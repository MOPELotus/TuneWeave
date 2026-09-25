use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests::{encrypted, requests, setup, setup_gated},
};
use tokio::sync::Notify;

const KEY: [u8; 8] = *b"17894932";
const PHONE: &str = "13800000000";
const CODE: &str = "24680";
const TM: &str = "server-token+&%1";
const SID: &str = "sms-native-session-42";
fn request() -> KuwoNativeSmsRequest {
    KuwoNativeSmsRequest {
        phone: PHONE.into(),
        allow_account_creation: true,
    }
}
fn sent() -> Vec<u8> {
    encrypted(&json!({"status":200,"tm":TM,"msg":"not returned"}))
}
fn accepted() -> serde_json::Value {
    json!({"result":"succ","sid":SID,"userInfo":{"uid":42,"nickName":"Listener"}})
}
fn validated() -> Vec<u8> {
    json_response(&json!({"result":"ok"}))
}
fn receipt() -> KuwoNativeSmsChallenge {
    KuwoNativeSmsChallenge {
        phone: PHONE.into(),
        server_tm: TM.into(),
        device: device::fixture_device(),
        deadline: Instant::now() + BUDGET,
        resend_at: Instant::now() + Duration::from_secs(60),
    }
}
fn assert_redacted(error: &TuneWeaveError) {
    let displayed = format!("{error:?} {error}");
    for secret in [PHONE, CODE, TM, SID, "upstream-secret-message"] {
        assert!(!displayed.contains(secret), "error reflected a secret");
    }
}
fn raw_encrypted(bytes: &[u8]) -> Vec<u8> {
    response(
        200,
        "application/json",
        "",
        &codec::fixture_response(bytes, &KEY),
    )
}

#[tokio::test]
async fn delivery_requires_explicit_creation_consent_and_valid_mainland_phone_before_network() {
    let fixture = setup(vec![]).await;
    let device = device::fixture_device();
    for phone in [
        "",
        "1380000000",
        "138000000000",
        "+8613800000000",
        "12800000000",
        "23800000000",
        "1380000000x",
        "１３８００００００００",
        " 13800000000",
        "13800000000\n",
    ] {
        let mut input = request();
        input.phone = phone.into();
        let error = fixture
            .client
            .send_native_login_sms(&input, &device)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_redacted(&error);
    }
    let mut input = request();
    input.allow_account_creation = false;
    let error = fixture
        .client
        .send_native_login_sms(&input, &device)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("explicit consent"));
    assert!(!KuwoNativeSmsRequest::default().allow_account_creation);
    assert!(!format!("{:?}", request()).contains(PHONE));
    assert_eq!(fixture.seen.len(), 0);
}

#[test]
fn sms_requests_match_independent_md5_and_native_cipher_reference_vectors() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    let send = send_query(&request(), &device::fixture_device(), "1789500000123", &KEY);
    let login = login_query(&receipt(), CODE, &KEY);
    for (kind, plain) in [("send", send), ("login", login)] {
        assert_eq!(plain, vectors[kind]["plain"].as_str().unwrap());
        assert_eq!(
            codec::seal_query(plain.as_bytes()).unwrap(),
            vectors[kind]["cipher_base64"].as_str().unwrap()
        );
    }
}

#[tokio::test]
async fn sms_login_binds_server_tm_and_device_then_independently_validates_identity() {
    let mut fixture = setup(vec![sent(), encrypted(&accepted()), validated()]).await;
    *fixture.client.web_session.lock().await = Some(KuwoWebSession {
        cookie_value: "anonymous-cookie-marker".into(),
        refresh_after: Instant::now() + BUDGET,
    });
    let device = device::fixture_device();
    let challenge = fixture
        .client
        .send_native_login_sms(&request(), &device)
        .await
        .unwrap();
    assert_eq!(challenge.phone, PHONE);
    assert_eq!(challenge.server_tm, TM);
    assert_eq!(challenge.device, device);
    assert!((290..=300).contains(&challenge.expires_in_secs()));
    assert!((59..=60).contains(&challenge.resend_after_secs()));
    for secret in [PHONE, TM, device.app_uid(), device.device_user()] {
        assert!(!format!("{challenge:?}").contains(secret));
    }
    fn require_send(_: impl std::future::Future + Send) {}
    require_send(fixture.client.login_native_sms(receipt(), CODE));
    let result = fixture
        .client
        .login_native_sms(challenge, CODE)
        .await
        .unwrap();
    assert!(result.profile.authenticated);
    assert_eq!(result.profile.account, "default");
    assert_eq!(result.profile.user_id.as_deref(), Some("42"));
    assert_eq!(result.profile.nickname.as_deref(), Some("Listener"));
    let credential = result.credential.unwrap();
    let native = credential::NativeCredential::parse(&credential)
        .unwrap()
        .input()
        .unwrap();
    assert_eq!(native.user_id(), "42");
    assert_eq!(native.session_id(), SID);
    assert_eq!(native.device_id(), device.app_uid());
    assert_eq!(native.device_user(), device.device_user());
    for secret in [PHONE, CODE, TM, "not returned", "anonymous-cookie-marker"] {
        assert!(!credential.secret().contains(secret));
        assert!(!format!("{:?}", result.profile).contains(secret));
    }
    let seen = requests(&mut fixture, 3).await;
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    assert!(seen[0].starts_with(&format!("GET {SEND_PATH}?f=ar&q=")));
    assert_eq!(
        seen[1].lines().next().unwrap(),
        format!(
            "GET {LOGIN_PATH}?f=ar&q={} HTTP/1.1",
            vectors["login"]["cipher_base64"].as_str().unwrap()
        )
    );
    assert!(
        seen[2].starts_with("GET /u.s?type=new_validate_ext&uid=42&sid=sms-native-session-42&")
    );
    assert!(seen[2].contains("android_id=ffeeddccbbaa49888776655443322110"));
    for r in &seen[..2] {
        assert!(
            r.lines()
                .any(|line| line.to_ascii_lowercase().starts_with("cookies:")
                    && line.contains(&password::device_metadata(&device)))
        );
    }
    for r in seen {
        assert!(!r.to_ascii_lowercase().contains("\r\ncookie:"));
        assert!(!r.to_ascii_lowercase().contains("\r\nsecret:"));
        assert!(!r.contains("anonymous-cookie-marker"));
    }
}

#[tokio::test]
async fn send_responses_require_unambiguous_success_and_a_bounded_server_receipt() {
    for tm in [
        json!("1789500000456"),
        json!(1789500000456_u64),
        json!("opaque+/%&=?token"),
    ] {
        let mut fixture = setup(vec![encrypted(&json!({"status":"200","tm":tm}))]).await;
        let challenge = fixture
            .client
            .send_native_login_sms(&request(), &device::fixture_device())
            .await
            .unwrap();
        assert!(!challenge.server_tm.is_empty());
        requests(&mut fixture, 1).await;
    }
    let bodies = [
        json!({}),
        json!({"status":200}),
        json!({"status":200,"tm":null}),
        json!({"status":"0200","tm":TM}),
        json!({"status":200.0,"tm":TM}),
        json!({"status":200,"tm":0}),
        json!({"status":200,"tm":-1}),
        json!({"status":200,"tm":1.5}),
        json!({"status":200,"tm":{}}),
        json!({"status":200,"tm":""}),
        json!({"status":200,"tm":"null"}),
        json!({"status":200,"tm":"undefined"}),
        json!({"status":200,"tm":"receipt\n"}),
        json!({"status":200,"tm":" receipt"}),
        json!({"status":200,"tm":"x".repeat(257)}),
        json!({"status":1108,"tm":TM,"msg":"upstream-secret-message"}),
        json!({"status":200,"tm":TM,"result":"fail"}),
        json!({"status":200,"tm":TM,"ret":"fail","result":"succ"}),
    ];
    let mut responses: Vec<_> = bodies.iter().map(encrypted).collect();
    responses.push(raw_encrypted(br#"{"status":200,"status":1108,"tm":"x"}"#));
    responses.push(raw_encrypted(br#"{"status":200,"tm":"x","tm":"y"}"#));
    for response in responses {
        let mut fixture = setup(vec![response]).await;
        let error = fixture
            .client
            .send_native_login_sms(&request(), &device::fixture_device())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.retryable);
        assert_redacted(&error);
        requests(&mut fixture, 1).await;
    }
}

#[tokio::test]
async fn malformed_or_ambiguous_login_and_independent_validation_failures_never_authenticate() {
    let mut bad = vec![
        json!({"result":"fail","status":6,"msg":"upstream-secret-message"}),
        json!({"result":"succ","sid":SID,"userInfo":{"uid":0}}),
        json!({"result":"succ","sid":SID,"userInfo":{"uid":"042"}}),
        json!({"result":"succ","sid":SID,"userInfo":{"uid":42},"status":1157}),
        json!({"result":"succ","ret":"fail","sid":SID,"userInfo":{"uid":42}}),
    ];
    for secret in [PHONE, CODE, TM] {
        let mut value = accepted();
        value["sid"] = json!(format!("prefix-{secret}"));
        bad.push(value);
        let mut value = accepted();
        value["userInfo"]["nickName"] = json!(format!("prefix-{secret}"));
        bad.push(value);
    }
    for body in bad {
        let mut fixture = setup(vec![sent(), encrypted(&body)]).await;
        let challenge = fixture
            .client
            .send_native_login_sms(&request(), &device::fixture_device())
            .await
            .unwrap();
        let error = fixture
            .client
            .login_native_sms(challenge, CODE)
            .await
            .unwrap_err();
        assert_redacted(&error);
        requests(&mut fixture, 2).await;
    }
    for body in [
        json!({"result":"fail","reason":"error_user_invalid"}),
        json!({"result":"fail","code":0,"msg":"Login_OK"}),
        json!({"result":"ok","reason":"error_user_invalid"}),
    ] {
        let mut fixture = setup(vec![sent(), encrypted(&accepted()), json_response(&body)]).await;
        let challenge = fixture
            .client
            .send_native_login_sms(&request(), &device::fixture_device())
            .await
            .unwrap();
        let error = fixture
            .client
            .login_native_sms(challenge, CODE)
            .await
            .unwrap_err();
        assert_redacted(&error);
        requests(&mut fixture, 3).await;
    }
}

#[tokio::test]
async fn invalid_code_and_expired_receipts_stop_before_io_and_late_results_are_not_accepted() {
    let fixture = setup(vec![]).await;
    for code in [
        "",
        "1234",
        "123456",
        "123a5",
        "１２３４５",
        " 12345",
        "12345\n",
    ] {
        let error = fixture
            .client
            .login_native_sms(receipt(), code)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_redacted(&error);
    }
    let mut expired = receipt();
    expired.deadline = Instant::now() - Duration::from_secs(1);
    assert_eq!(expired.expires_in_secs(), 0);
    expired.resend_at = expired.deadline;
    assert_eq!(expired.resend_after_secs(), 0);
    assert_eq!(
        fixture
            .client
            .login_native_sms(expired, CODE)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(fixture.seen.len(), 0);

    for validation_boundary in [false, true] {
        let gate = Arc::new(Notify::new());
        let responses = if validation_boundary {
            vec![
                (encrypted(&accepted()), None),
                (validated(), Some(gate.clone())),
            ]
        } else {
            vec![(encrypted(&accepted()), Some(gate.clone()))]
        };
        let mut fixture = setup_gated(responses).await;
        let mut challenge = receipt();
        // Leave enough time to reach the gated loopback boundary when the full
        // test suite is running concurrently. The response remains withheld
        // until expiry, so this still exercises rejection of late results.
        challenge.deadline = Instant::now() + Duration::from_secs(1);
        let client = fixture.client.clone();
        let task = tokio::spawn(async move { client.login_native_sms(challenge, CODE).await });
        let count = if validation_boundary { 2 } else { 1 };
        for _ in 0..count {
            tokio::time::timeout(Duration::from_secs(2), fixture.seen.recv())
                .await
                .unwrap()
                .unwrap();
        }
        let error = task.await.unwrap().unwrap_err();
        assert!(matches!(
            error.code,
            ErrorCode::Conflict | ErrorCode::UpstreamError
        ));
        assert!(!error.retryable);
        assert_redacted(&error);
        gate.notify_one();
        requests(&mut fixture, 0).await;
    }
}

#[tokio::test]
async fn cancellation_at_send_login_or_validation_does_not_deliver_a_receipt_or_credential() {
    for boundary in 0..3 {
        let gate = Arc::new(Notify::new());
        let all = [sent(), encrypted(&accepted()), validated()];
        let responses = all
            .into_iter()
            .enumerate()
            .take(boundary + 1)
            .map(|(i, body)| (body, (i == boundary).then(|| gate.clone())))
            .collect();
        let mut fixture = setup_gated(responses).await;
        let client = fixture.client.clone();
        let task = tokio::spawn(async move {
            let receipt = client
                .send_native_login_sms(&request(), &device::fixture_device())
                .await?;
            client.login_native_sms(receipt, CODE).await
        });
        for _ in 0..=boundary {
            tokio::time::timeout(Duration::from_secs(2), fixture.seen.recv())
                .await
                .unwrap()
                .unwrap();
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        gate.notify_one();
        requests(&mut fixture, 0).await;
    }
}

#[tokio::test]
async fn delivery_does_not_retry_http_errors_redirects_or_invalid_payloads() {
    let responses = [
        response(
            302,
            "application/json",
            "Location: https://example.invalid/\r\n",
            b"{}",
        ),
        response(
            429,
            "application/json",
            "Retry-After: 90\r\n",
            b"upstream-secret-message",
        ),
        response(503, "application/json", "", b"upstream-secret-message"),
        response(200, "text/html", "", b"upstream-secret-message"),
        response(200, "application/json", "", b"not-base64"),
        response(
            200,
            "application/json",
            "",
            &vec![b'A'; codec::MAX_RESPONSE + 1],
        ),
    ];
    for body in responses {
        let mut fixture = setup(vec![body]).await;
        let error = fixture
            .client
            .send_native_login_sms(&request(), &device::fixture_device())
            .await
            .unwrap_err();
        assert!(!error.retryable);
        assert_redacted(&error);
        requests(&mut fixture, 1).await;
    }
}

#[tokio::test]
async fn separate_phone_receipts_can_complete_in_reverse_order_without_changing_identity() {
    let mut second_login = accepted();
    second_login["sid"] = json!("sms-native-session-43");
    second_login["userInfo"]["uid"] = json!(43);
    let mut fixture = setup(vec![
        sent(),
        encrypted(&json!({"status":200,"tm":"second-server-tm"})),
        encrypted(&second_login),
        validated(),
        encrypted(&accepted()),
        validated(),
    ])
    .await;
    let first = fixture
        .client
        .send_native_login_sms(&request(), &device::fixture_device())
        .await
        .unwrap();
    let mut second = request();
    second.phone = "13900000000".into();
    let second = fixture
        .client
        .send_native_login_sms(&second, &device::fixture_device())
        .await
        .unwrap();
    let second_wire = login_query(&second, "13579", &KEY);
    let second_result = fixture
        .client
        .login_native_sms(second, "13579")
        .await
        .unwrap();
    let first_result = fixture.client.login_native_sms(first, CODE).await.unwrap();
    assert_eq!(second_result.profile.user_id.as_deref(), Some("43"));
    assert_eq!(first_result.profile.user_id.as_deref(), Some("42"));
    assert_ne!(first_result.credential, second_result.credential);
    let seen = requests(&mut fixture, 6).await;
    assert_eq!(
        seen[2].lines().next().unwrap(),
        format!(
            "GET {LOGIN_PATH}?f=ar&q={} HTTP/1.1",
            codec::seal_query(second_wire.as_bytes()).unwrap()
        )
    );
}
