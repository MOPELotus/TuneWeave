use super::*;

fn callback(encoded: &str) -> String {
    json!({"status":1,"error_code":0,"vType":2,"error_msg":"private-message","verify_data":encoded})
        .to_string()
}

#[test]
fn sms_browser_callback_decodes_once_preserves_plus_and_never_accepts_header_injection() {
    let proof = Proof::parse(&callback("opaque%2Btoken+literal%252F%3D")).unwrap();
    let header = proof.header.unwrap();
    assert_eq!(header.to_str().unwrap(), "opaque+token+literal%2F=");
    assert!(header.is_sensitive());
    for value in [
        "bad%",
        "%GG",
        "%0d%0aCookie%3Ax",
        "%00",
        "%09",
        "%7f",
        "%ff",
        "%C3%A9",
        "%20x",
        "x%20",
    ] {
        let e = Proof::parse(&callback(value)).err().unwrap();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(!format!("{e:?}").contains("private-message"));
    }
    assert!(Proof::parse(&callback(&"x".repeat(8193))).is_err());
    assert!(Proof::parse(&"x".repeat(16385)).is_err());
    for value in ["", "null", "undefined"] {
        let proof = Proof::parse(&callback(value)).unwrap();
        assert_eq!(proof.header.is_none(), value.is_empty());
        assert!(proof.reject_reflection(value.as_bytes()).is_ok());
    }
}

#[test]
fn sms_browser_callback_rejects_ambiguous_or_failed_json_and_reflected_proof() {
    for value in [
        json!({"status":0,"error_code":0,"vType":2,"verify_data":"proof"}),
        json!({"status":1,"error_code":10,"vType":2,"verify_data":"proof"}),
        json!({"status":"1","error_code":0,"vType":2,"verify_data":"proof"}),
        json!({"status":1,"error_code":0,"vType":2,"verify_data":null}),
        json!({"status":1,"error_code":0,"vType":2,"verify_data":"proof","userid":222}),
        json!({"type":"kgVerifyCallbackData","dataJson":callback("proof")}),
    ] {
        assert!(Proof::parse(&value.to_string()).is_err());
    }
    assert!(
        Proof::parse(
            r#"{"status":1,"error_code":0,"vType":2,"verify_data":"a","verify_data":"b"}"#
        )
        .is_err()
    );
    let proof = Proof::parse(&callback("private%2Bproof%3D")).unwrap();
    for reflected in ["private+proof=", "private%2Bproof%3D"] {
        assert!(proof.reject_reflection(reflected.as_bytes()).is_err());
    }
    let escaped = Proof::parse(&callback("private%22proof%5C")).unwrap();
    assert!(
        escaped
            .reject_reflection(br#"{"nested":[{"nickname":"private\"proof\\"}]}"#)
            .is_err()
    );
}

#[test]
fn sms_browser_page_uses_only_fixed_origin_and_the_original_event_and_mid() {
    let receipt = KugouWebSmsChallenge {
        request: KugouWebSmsRequest {
            phone: "13800000000".into(),
            allow_account_creation: false,
        },
        device: KugouDevice::default().identity().into_web(),
        deadline: Deadline::now() + Duration::from_secs(300),
        attempts: 2,
        password_principal: None,
        state: Stage::Waiting,
    };
    let reply = |data: Value| {
        serde_json::from_value::<Envelope>(json!({"status":0,"error_code":20028,"data":data}))
            .unwrap()
    };
    let challenge = challenge(
        &reply(json!(
            "url=https://evil.test&eventid=opaque%2Fid%3Fx%3D1+plus&token=private"
        )),
        &receipt,
    )
    .unwrap();
    assert_eq!(
        challenge.url,
        format!(
            "{ORIGIN}/apps/verify-h5/dist/#/index/opaque%2Fid%3Fx%3D1%2Bplus/1014/null/{}/TuneWeaveVerify",
            receipt.device.mid
        )
    );
    assert_eq!(challenge.remaining_attempts, 3);
    assert!(!challenge.url.contains("evil") && !challenge.url.contains("private"));
    assert!(!format!("{challenge:?}").contains("opaque"));
    for data in [
        json!(null),
        json!({"eventid":"one"}),
        json!("eventid="),
        json!("eventid=x&eventid=y"),
        json!("eventid=%0a"),
        json!("eventid=%"),
        json!("eventid=".to_owned() + &"a".repeat(513)),
        json!("x=".to_owned() + &"a".repeat(4096)),
    ] {
        assert!(super::challenge(&reply(data), &receipt).is_err());
    }
}
