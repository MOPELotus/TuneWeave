use super::*;
use reqwest::header::{HeaderMap, SET_COOKIE};

fn request() -> PasswordLoginRequest {
    PasswordLoginRequest {
        backend: Default::default(),
        account: "default".into(),
        principal_type: PrincipalType::Username,
        principal: "synthetic-user".into(),
        password: "synthetic 密码+pass".into(),
        password_format: PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    }
}
fn headers() -> HeaderMap {
    let mut value = HeaderMap::new();
    value.insert(
        SET_COOKIE,
        "KuGoo=KugooID=111&t=synthetic-token&a_id=1014; Domain=.kugou.com; Path=/"
            .parse()
            .unwrap(),
    );
    value
}

#[test]
fn password_encryption_and_query_match_independent_official_consumer_vectors() {
    let v: Value = serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    let device: KugouDeviceIdentity = serde_json::from_value(v["device"].clone()).unwrap();
    let cipher = crypto::WebCipher::test_cipher();
    for case in v["cases"].as_array().unwrap() {
        let mut request = request();
        request.principal = case["principal"].as_str().unwrap().into();
        request.principal_type = serde_json::from_value(case["principal_type"].clone()).unwrap();
        validate(&request).unwrap();
        let actual = parameters(&request, &device, 1700000000123, &cipher).unwrap();
        assert_eq!(serde_json::to_value(actual).unwrap(), case["query"]);
    }
}

#[test]
fn password_inputs_are_explicit_and_never_silently_trimmed_or_md5_hashed() {
    let valid = request();
    validate(&valid).unwrap();
    let mut inputs = vec![];
    for password in ["", " leading", "trailing ", "line\nbreak"] {
        let mut r = valid.clone();
        r.password = password.into();
        inputs.push(r);
    }
    let mut r = valid.clone();
    r.password = "p".repeat(1025);
    inputs.push(r);
    let mut r = valid.clone();
    r.password_format = PasswordFormat::Md5;
    inputs.push(r);
    let mut r = valid.clone();
    r.secure_captcha = Some("ticket".into());
    inputs.push(r);
    for principal in ["", " padded", "line\nbreak"] {
        let mut r = valid.clone();
        r.principal = principal.into();
        inputs.push(r);
    }
    let mut r = valid.clone();
    r.country_code = Some("86".into());
    inputs.push(r);
    for principal in [
        "10000000000",
        "1380000000",
        "+8613800000000",
        "1e10",
        "abcdefghijk",
    ] {
        let mut r = valid.clone();
        r.principal_type = PrincipalType::Phone;
        r.principal = principal.into();
        inputs.push(r);
    }
    for principal in ["no-at", "@host", "name@", "name@a@b", "a b@c"] {
        let mut r = valid.clone();
        r.principal_type = PrincipalType::Email;
        r.principal = principal.into();
        inputs.push(r);
    }
    for invalid in inputs {
        assert!(validate(&invalid).is_err());
    }
    for country in [None, Some("86".into()), Some("+86".into())] {
        let mut r = valid.clone();
        r.principal_type = PrincipalType::Phone;
        r.principal = "13800000000".into();
        r.country_code = country;
        validate(&r).unwrap();
    }
}

#[test]
fn password_success_requires_matching_json_and_cookie_uid_and_errors_hide_contact_data() {
    let device = KugouDevice::default().identity().into_web();
    for userid in [json!(111), json!("111")] {
        let bytes=serde_json::to_vec(&json!({"status":1,"error_code":0,"data":{"userid":userid,"username":"hidden-email@example.test"}})).unwrap();
        let session = parse(&headers(), &bytes, &device).unwrap();
        assert_eq!(session.user_id, "111");
        assert!(!format!("{session:?}").contains("hidden-email"));
    }
    for data in [
        json!({}),
        json!({"userid":null}),
        json!({"userid":0}),
        json!({"userid":"0111"}),
        json!({"userid":"222"}),
    ] {
        assert!(
            parse(
                &headers(),
                &serde_json::to_vec(&json!({"status":1,"error_code":0,"data":data})).unwrap(),
                &device
            )
            .is_err()
        );
    }
    assert!(
        parse(
            &headers(),
            br#"{"status":1,"error_code":0,"data":{"userid":111,"userid":111}}"#,
            &device
        )
        .is_err()
    );
    for (code, kind) in [
        (20020, Some("image")),
        (20021, Some("image")),
        (30791, Some("interactive")),
        (30767, Some("phone")),
        (30768, Some("phone")),
        (30798, Some("binding")),
        (30733, Some("binding")),
        (34172, Some("binding")),
        (30701, None),
        (30702, None),
        (30703, None),
    ] {
        let bytes = serde_json::to_vec(
            &json!({"status":0,"error_code":code,"data":"13800000000 synthetic-password"}),
        )
        .unwrap();
        let mut error = parse(&headers(), &bytes, &device).unwrap_err();
        assert_eq!(
            error.code,
            if kind.is_some() {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::AuthenticationRequired
            }
        );
        assert!(error.take_caller_credential_update().is_none());
        for secret in ["13800000000", "synthetic-password"] {
            assert!(!format!("{error:?}").contains(secret));
        }
    }
}
