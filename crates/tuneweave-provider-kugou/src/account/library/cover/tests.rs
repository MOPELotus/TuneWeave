use super::*;

#[test]
fn standard_authorization_uses_custom_bucket_and_empty_body_signature() {
    let session = crate::account::tests::session(KugouLoginClient::Standard);
    let mut params = authorization_parameters(&session, 1_700_000_000);
    params.insert("mid", "fixture-mid".into());
    assert_eq!(params["bucket"], "custom");
    assert_eq!(params["method"], "POST");
    assert_eq!(params["uuid"], "-");
    assert_eq!(params["userid"], "123456789");
    // Independent Python hashlib vectors, including sorted exact query fields.
    assert_eq!(params["buVerifyCode"], "e47fa78251f48f77e282c7ad019b7e51");
    assert_eq!(
        android_signature(&params, b""),
        "f378e24650a7815925da1b3bb6201d59"
    );
}

#[test]
fn concept_cover_authorization_uses_its_version_uuid_and_signature_without_standard_method() {
    let session = crate::account::tests::session(KugouLoginClient::Concept);
    let mut params = authorization_parameters(&session, 1_700_000_000);
    params.insert("mid", "fixture-mid".into());
    assert_eq!(params.len(), 12);
    assert_eq!(params["appid"], "3116");
    assert_eq!(params["clientver"], "11490");
    assert_eq!(params["bucket"], "custom");
    assert_eq!(params["uuid"], "-");
    assert_eq!(params["userid"], "123456789");
    assert!(!params.contains_key("method"));
    // Independent Python hashlib vectors for this exact Concept query.
    assert_eq!(params["buVerifyCode"], "a77f87462bdcf7db191809faa5b3cef2");
    assert_eq!(
        concept_signature(&params, b""),
        "7578ad1f8c8b6640d5e679b79ea2c5f9"
    );
}

#[test]
fn concept_cover_authorization_fallback_preserves_element_zero_and_never_selects_another_ticket() {
    let encode = |value: Value| {
        serde_json::to_vec(&json!({"status":1,"error_code":0,"data":value})).unwrap()
    };
    for value in [
        json!({"authorization":"fixture+/= token","authorizations":["other"]}),
        json!({"authorization":"","authorizations":["fixture+/= token","other"]}),
        json!({"authorizations":["fixture+/= token"]}),
    ] {
        assert_eq!(
            authorization(&encode(value), KugouLoginClient::Concept)
                .unwrap()
                .0,
            "fixture+/= token"
        );
    }
    for value in [
        json!({"authorizations":[]}),
        json!({"authorizations":["","other"]}),
        json!({"authorizations":[null,"other"]}),
        json!({"authorization":"bad\nsecret","authorizations":["other"]}),
        json!({"authorizations":[" ","other"]}),
        json!({"authorizations":["x".repeat(16_385)]}),
        json!({"authorizations":vec!["ticket";65]}),
    ] {
        let failure = match authorization(&encode(value), KugouLoginClient::Concept) {
            Ok(_) => panic!("accepted invalid Concept upload authorization"),
            Err(error) => error,
        };
        assert!(!format!("{failure:?}").contains("secret"));
    }
    assert!(
        authorization(
            &encode(json!({"authorizations":["ticket"]})),
            KugouLoginClient::Standard
        )
        .is_err()
    );
}

#[test]
fn concept_cover_upload_binds_local_day_and_epoch_once_across_midnight_and_offsets() {
    let session = crate::account::tests::session(KugouLoginClient::Concept);
    // The first three represent the same epoch in distinct request environments.
    // Hashes were independently computed with Python hashlib, without changing TZ.
    for (instant, digest) in [
        (
            "2026-09-22T20:00:00-04:00",
            "c7083e283ac4845917fd68b1126d010f",
        ),
        (
            "2026-09-23T00:00:00+00:00",
            "f120d17b2c80927095717ce5a2b8dc59",
        ),
        (
            "2026-09-23T08:00:00+08:00",
            "f120d17b2c80927095717ce5a2b8dc59",
        ),
        (
            "2026-09-23T23:59:59+08:00",
            "f120d17b2c80927095717ce5a2b8dc59",
        ),
        (
            "2026-09-24T00:00:00+08:00",
            "3e9b0e812648d4ff1333ba52f3b09a9d",
        ),
        (
            "2026-12-31T23:59:59-12:00",
            "c35dcfe377a873eb7e50814c20ca4a6e",
        ),
        (
            "2027-01-01T00:00:00-12:00",
            "4713e7445af3fcb3bf54fe8d80e0937f",
        ),
    ] {
        let time = chrono::DateTime::parse_from_rfc3339(instant).unwrap();
        let params = concept_upload_parameters(
            &session,
            CoverAuthorization("opaque+/= ticket".into()),
            time,
        )
        .unwrap();
        assert_eq!(params["md5"], digest, "{instant}");
        assert_eq!(params["clienttime"], time.timestamp().to_string());
        assert_eq!(params["authorization"], "opaque+/= ticket");
        assert_eq!(params["clientver"], "11490");
        assert_eq!(params["userid"], session.user_id);
        assert_eq!(params["token"], session.token);
        assert_eq!(params["uuid"], "-");
        assert_eq!(params["body_empty"], "1");
        assert_eq!(params["extendName"], ".jpg");
        assert!(!params.contains_key("iscovered"));
        assert!(!params.contains_key("method"));
    }
    assert!(
        concept_upload_parameters(
            &session,
            CoverAuthorization("ticket".into()),
            chrono::DateTime::parse_from_rfc3339("1969-12-31T23:59:59+00:00").unwrap(),
        )
        .is_err()
    );
}

#[test]
fn uploaded_cover_requires_boolean_success_and_a_relative_filename() {
    assert_eq!(
        uploaded(br#"{"IsSuccess":true,"FileName":"20260923/abc_123.jpg"}"#)
            .unwrap()
            .filename(),
        "20260923/abc_123.jpg"
    );
    for value in [
        json!({"IsSuccess":false,"FileName":"ok.jpg","Message":"secret"}),
        json!({"IsSuccess":1,"FileName":"ok.jpg"}),
        json!({"IsSuccess":true}),
        json!({"IsSuccess":true,"FileName":"../secret"}),
        json!({"IsSuccess":true,"FileName":"/absolute.jpg"}),
        json!({"IsSuccess":true,"FileName":"https://evil.test/a.jpg"}),
        json!({"IsSuccess":true,"FileName":"a.jpg?token=secret"}),
    ] {
        let failure = uploaded(&serde_json::to_vec(&value).unwrap())
            .err()
            .unwrap();
        assert!(!format!("{failure:?}").contains("secret"));
    }
}
