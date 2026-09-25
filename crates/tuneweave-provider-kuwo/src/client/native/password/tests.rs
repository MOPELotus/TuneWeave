use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests::{encrypted, requests, setup},
};

const KEY: [u8; 8] = *b"17894932";
const SID: &str = "native-session-42";
fn request() -> PasswordLoginRequest {
    PasswordLoginRequest {
        backend: Default::default(),
        account: "default".into(),
        principal_type: PrincipalType::Username,
        principal: "听众+&42".into(),
        password: " leading秘%+& word ".into(),
        password_format: PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    }
}
fn accepted() -> serde_json::Value {
    json!({"result":"succ","sid":SID,"userInfo":{"uid":42,"nickName":"Listener","password":"ignored-password-field","pwdEmail":"ignored@example.invalid","vip":true},"other":"ignored"})
}
fn validation() -> Vec<u8> {
    json_response(&json!({"result":"ok"}))
}
fn parsed_credential(value: serde_json::Value) -> ProviderCredential {
    ProviderCredential::new(Platform::Kuwo, "kuwo_native_v1", value.to_string(), None).unwrap()
}

#[tokio::test]
async fn phone_and_email_password_use_the_native_password_wire_and_verify_identity() {
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    let (_, suffix) = vector["plain"].as_str().unwrap().split_once('&').unwrap();
    for (kind, principal, encoded) in [
        (PrincipalType::Phone, "13800138000", "13800138000"),
        (
            PrincipalType::Email,
            "Listener+Tag@Example.invalid",
            "Listener%2BTag%40Example.invalid",
        ),
    ] {
        let mut r = request();
        r.principal_type = kind;
        r.principal = principal.into();
        let device = device::fixture_device();
        let mut fixture = setup(vec![encrypted(&accepted()), validation()]).await;
        let result = fixture
            .client
            .login_native_password(&r, &device)
            .await
            .unwrap();
        assert_eq!(result.profile.user_id.as_deref(), Some("42"));
        let credential = result.credential.unwrap();
        assert!(!credential.secret().contains(principal));
        assert!(!credential.secret().contains(&r.password));
        let seen = requests(&mut fixture, 2).await;
        let expected = format!("username={encoded}&{suffix}");
        assert_eq!(password_query(&r, &device, &KEY), expected);
        assert_eq!(
            seen[0].lines().next().unwrap(),
            format!(
                "GET {PATH}?f=ar&q={} HTTP/1.1",
                codec::seal_query(expected.as_bytes()).unwrap()
            )
        );
        assert!(seen[1].contains("uid=42") && seen[1].contains("sid=native-session-42"));
        assert!(
            !seen
                .iter()
                .any(|request| request.contains("login_sms") || request.contains("get_sms"))
        );
    }
}

#[tokio::test]
async fn typed_password_principals_reject_invalid_identifiers_before_io() {
    let fixture = setup(vec![]).await;
    let device = device::fixture_device();
    for (kind, principal, country) in [
        (PrincipalType::Phone, "12800138000", None),
        (PrincipalType::Phone, "1380013800", None),
        (PrincipalType::Phone, "+8613800138000", None),
        (PrincipalType::Phone, "１３８００１３８０００", None),
        (PrincipalType::Phone, "13800138000", Some("1")),
        (PrincipalType::Email, "@example.invalid", None),
        (PrincipalType::Email, "listener@", None),
        (PrincipalType::Email, "listener@@example.invalid", None),
        (PrincipalType::Email, "listener name@example.invalid", None),
        (PrincipalType::Email, "listener@example.invalid", Some("86")),
    ] {
        let mut r = request();
        r.principal_type = kind;
        r.principal = principal.into();
        r.country_code = country.map(str::to_owned);
        assert_eq!(
            fixture
                .client
                .login_native_password(&r, &device)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(fixture.seen.len(), 0);
}

#[tokio::test]
async fn unsupported_or_invalid_password_input_stops_before_io() {
    let device = device::fixture_device();
    let fixture = setup(vec![]).await;
    let mut invalids = Vec::new();
    for principal in ["", " leading", "trailing ", "line\nbreak"] {
        let mut r = request();
        r.principal = principal.into();
        invalids.push(r);
    }
    for password in [String::new(), "p\0ass".into(), "x".repeat(1025)] {
        let mut r = request();
        r.password = password;
        invalids.push(r);
    }
    let mut r = request();
    r.principal = "x".repeat(257);
    invalids.push(r);
    let mut r = request();
    r.password_format = PasswordFormat::Md5;
    invalids.push(r);
    for kind in [PrincipalType::Phone, PrincipalType::Email] {
        let mut r = request();
        r.principal_type = kind;
        invalids.push(r);
    }
    let mut r = request();
    r.country_code = Some("86".into());
    invalids.push(r);
    let mut r = request();
    r.secure_captcha = Some("unexpected".into());
    invalids.push(r);
    let mut r = request();
    r.account = "server-alias".into();
    invalids.push(r);
    for r in invalids {
        assert_eq!(
            fixture
                .client
                .login_native_password(&r, &device)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(fixture.seen.len(), 0);
}

#[tokio::test]
async fn password_login_preserves_utf8_and_spaces_then_independently_checks_the_returned_identity()
{
    let device = device::fixture_device();
    let request = request();
    let mut fixture = setup(vec![encrypted(&accepted()), validation(), validation()]).await;
    *fixture.client.web_session.lock().await = Some(KuwoWebSession {
        cookie_value: "anonymous-marker".into(),
        refresh_after: Instant::now() + Duration::from_secs(600),
    });
    fn require_send(_: impl std::future::Future + Send) {}
    require_send(fixture.client.login_native_password(&request, &device));
    let result = fixture
        .client
        .login_native_password(&request, &device)
        .await
        .unwrap();
    assert!(result.profile.authenticated);
    assert_eq!(result.profile.account, "default");
    assert_eq!(result.profile.user_id.as_deref(), Some("42"));
    assert_eq!(result.profile.nickname.as_deref(), Some("Listener"));
    assert!(result.profile.avatar_url.is_none());
    assert!(result.profile.extensions.is_empty());
    let credential = result.credential.unwrap();
    assert_eq!(credential.kind, "kuwo_native_v1");
    assert!(credential.expires_at.is_none());
    for secret in [
        &request.password,
        &BASE64_STANDARD.encode(request.password.as_bytes()),
        "ignored-password-field",
        "ignored@example.invalid",
        "anonymous-marker",
    ] {
        assert!(!credential.secret().contains(secret));
    }
    let parsed = NativeCredential::parse(&credential).unwrap();
    let session = parsed.input().unwrap();
    assert_eq!(session.user_id(), "42");
    assert_eq!(session.session_id(), SID);
    assert_eq!(session.device_id(), device.app_uid());
    assert_eq!(session.device_user(), device.device_user());
    assert!(!format!("{credential:?} {parsed:?} {session:?}").contains(SID));
    require_send(fixture.client.validate_native_login(&credential));
    let profile = fixture
        .client
        .validate_native_login(&credential)
        .await
        .unwrap();
    assert_eq!(profile.user_id.as_deref(), Some("42"));
    assert!(profile.nickname.is_none());
    assert!(profile.authenticated);
    let seen = requests(&mut fixture, 3).await;
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    assert_eq!(
        password_query(&request, &device, &KEY),
        vector["plain"].as_str().unwrap()
    );
    assert_eq!(
        seen[0].lines().next().unwrap(),
        format!(
            "GET {PATH}?f=ar&q={} HTTP/1.1",
            vector["cipher_base64"].as_str().unwrap()
        )
    );
    let native_header = seen[0]
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("cookies:"))
        .unwrap();
    assert!(native_header.contains(&device_metadata(&device)));
    assert!(native_header.contains("loginUid=0,loginSid=0,"));
    assert!(!native_header.contains(SID));
    for r in &seen {
        assert!(!r.to_ascii_lowercase().contains("\r\ncookie:"));
        assert!(!r.to_ascii_lowercase().contains("\r\nsecret:"));
        assert!(!r.contains("anonymous-marker"));
    }
    for r in &seen[1..] {
        let url = Url::parse(&format!(
            "http://localhost{}",
            r.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        let q: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["uid"], "42");
        assert_eq!(q["sid"], SID);
        assert_eq!(q["appuid"], device.app_uid());
        assert_eq!(q["android_id"], device.android_id());
    }
    assert_eq!(
        fixture
            .client
            .web_session
            .lock()
            .await
            .as_ref()
            .unwrap()
            .cookie_value,
        "anonymous-marker"
    );
}

#[tokio::test]
async fn independent_validation_failure_never_delivers_an_authenticated_result_or_credential() {
    for reply in [
        json_response(&json!({"result":"fail","reason":"error_user_invalid"})),
        json_response(&json!({"code":0,"msg":"Login_OK"})),
        response(503, "application/json", "", b"{}"),
        response(
            302,
            "application/json",
            "Location: https://example.invalid/\r\n",
            b"{}",
        ),
    ] {
        let mut fixture = setup(vec![encrypted(&accepted()), reply]).await;
        let mut error = fixture
            .client
            .login_native_password(&request(), &device::fixture_device())
            .await
            .unwrap_err();
        assert!(matches!(
            error.code,
            ErrorCode::AuthenticationRequired | ErrorCode::UpstreamError
        ));
        assert!(!format!("{error:?}").contains(SID));
        assert!(error.take_caller_credential_update().is_none());
        requests(&mut fixture, 2).await;
    }
}

#[test]
fn password_response_rejects_inconsistent_success_and_reflected_password_or_sid() {
    let device = device::fixture_device();
    let request = request();
    let mut bad = vec![
        json!({}),
        json!({"result":"succ"}),
        json!({"result":"fail","status":1001,"msg":"parameter error"}),
    ];
    for change in [
        json!({"uid":0}),
        json!({"uid":"042"}),
        json!({"uid":2147483648u64}),
        json!({"uid":42,"nickName":SID}),
        json!({"uid":42,"nickName":request.password}),
        json!({"uid":42,"nickName":BASE64_STANDARD.encode(request.password.as_bytes())}),
    ] {
        let mut v = accepted();
        v["userInfo"] = change;
        bad.push(v);
    }
    for (field, value) in [
        ("ret", json!("fail")),
        ("status", json!(1159)),
        ("status", json!(1157)),
        ("status", json!(1136)),
        ("sid", json!("")),
        (
            "sid",
            json!(BASE64_STANDARD.encode(request.password.as_bytes())),
        ),
    ] {
        let mut v = accepted();
        v[field] = value;
        bad.push(v);
    }
    for v in bad {
        assert!(parse_password(&serde_json::to_vec(&v).unwrap(), &request, &device).is_err());
    }
    for raw in [
        r#"{"result":"succ","sid":"a-session","userInfo":{"uid":42,"uid":43}}"#,
        r#"{"result":"succ","result":"fail","sid":"a-session","userInfo":{"uid":42}}"#,
    ] {
        assert!(parse_password(raw.as_bytes(), &request, &device).is_err());
    }
    // A short password sharing incidental SID characters is not reflected data.
    let mut short = request.clone();
    short.password = "a".into();
    assert!(parse_password(&serde_json::to_vec(&accepted()).unwrap(), &short, &device).is_ok());
}

#[tokio::test]
async fn password_transport_and_platform_errors_are_terminal_without_validation_or_retries() {
    for reply in [
        encrypted(&json!({"result":"fail","status":1136,"enum":"3"})),
        encrypted(&json!({"result":"fail","status":1157})),
        response(403, "application/json", "", b"{}"),
        response(429, "application/json", "Retry-After: 9\r\n", b"{}"),
        response(200, "text/html", "", b"not-json"),
        response(
            302,
            "application/json",
            "Location: https://example.invalid\r\n",
            b"{}",
        ),
    ] {
        let mut fixture = setup(vec![reply]).await;
        let mut error = fixture
            .client
            .login_native_password(&request(), &device::fixture_device())
            .await
            .unwrap_err();
        assert!(matches!(
            error.code,
            ErrorCode::AuthenticationRequired
                | ErrorCode::PermissionDenied
                | ErrorCode::RateLimited
                | ErrorCode::UpstreamError
        ));
        assert!(error.take_caller_credential_update().is_none());
        requests(&mut fixture, 1).await;
    }
}

#[tokio::test]
async fn caller_credentials_require_strict_schema_and_fresh_identity_validation() {
    let device = device::fixture_device();
    let input = device.session_input("42", SID).unwrap();
    let a = NativeCredential::verified(&input)
        .unwrap()
        .caller()
        .unwrap();
    let b = NativeCredential::verified(&input)
        .unwrap()
        .caller()
        .unwrap();
    assert_ne!(a.secret(), b.secret());
    let mut inconsistent = input.clone();
    inconsistent.device_user = device.android_id().into();
    assert!(NativeCredential::verified(&inconsistent).is_err());
    let seed: serde_json::Value = serde_json::from_str(a.secret()).unwrap();
    let mut invalids = Vec::new();
    for (field, value) in [
        ("version", json!(2)),
        ("generation", json!("short")),
        ("user_id", json!("0")),
        ("user_id", json!("042")),
        ("app_uid", json!("042")),
        ("session_id", json!("0")),
        ("password", json!("unexpected")),
    ] {
        let mut v = seed.clone();
        v[field] = value;
        invalids.push(parsed_credential(v));
    }
    let mut v = seed.clone();
    v["context"]["android_id"] = v["context"]["device_user"].clone();
    invalids.push(parsed_credential(v));
    invalids.push(ProviderCredential::new(Platform::Kugou, &a.kind, a.secret(), None).unwrap());
    invalids.push(ProviderCredential::new(Platform::Kuwo, "cookie", a.secret(), None).unwrap());
    invalids.push(
        ProviderCredential::new(Platform::Kuwo, &a.kind, a.secret(), Some(u64::MAX)).unwrap(),
    );
    invalids
        .push(ProviderCredential::new(Platform::Kuwo, &a.kind, "x".repeat(16385), None).unwrap());
    invalids.push(
        ProviderCredential::new(
            Platform::Kuwo,
            &a.kind,
            a.secret()
                .replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
            None,
        )
        .unwrap(),
    );
    let fixture = setup(vec![]).await;
    for credential in invalids {
        assert_eq!(
            fixture
                .client
                .validate_native_login(&credential)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(fixture.seen.len(), 0);
    let mut fixture = setup(vec![json_response(
        &json!({"result":"fail","reason":"error_user_invalid"}),
    )])
    .await;
    assert_eq!(
        fixture
            .client
            .validate_native_login(&a)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    requests(&mut fixture, 1).await;
}

#[tokio::test]
#[ignore = "official Kuwo empty-identity login denial; no account or password is submitted"]
async fn live_native_empty_password_request_is_explicitly_denied() {
    let client = KuwoClient::new(&KuwoConfig::default()).unwrap();
    let device = KuwoNativeDeviceStore::default()
        .initialize(&client)
        .await
        .unwrap();
    let mut request = request();
    request.principal.clear();
    request.password.clear();
    let key = client.native_response_key().unwrap();
    let plain = password_query(&request, &device, &key);
    let cipher = codec::seal_query(plain.as_bytes()).unwrap();
    let target = format!(
        "{}?f=ar&q={cipher}",
        client.native_target(EXCHANGE_HOST, PATH)
    );
    let body = client
        .native_get_with_metadata(
            EXCHANGE_HOST,
            PATH,
            "native_password_anonymous_probe",
            target,
            Some(device_metadata(&device)),
            |bytes| codec::open_response(bytes, &key),
        )
        .await
        .unwrap();
    let parsed: ExchangeBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed.result.as_deref(), Some("fail"));
    assert_eq!(parsed.status.as_deref(), Some("1001"));
    assert!(parsed.sid.is_none());
    assert!(parsed.user_info.is_none());
}

#[tokio::test]
async fn cancelling_either_login_boundary_does_not_finish_or_start_the_next_request() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{Notify, mpsc},
    };
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    for cancelled_boundary in 0..2 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let (tx, mut seen) = mpsc::unbounded_channel();
        let gate = Arc::new(Notify::new());
        let release = gate.clone();
        let mut server = Server(tokio::spawn(async move {
            for (index, reply) in [encrypted(&accepted()), validation()]
                .into_iter()
                .enumerate()
                .take(cancelled_boundary + 1)
            {
                let (mut stream, _) =
                    tokio::time::timeout(Duration::from_secs(3), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut bytes = [0; 1024];
                    let n = stream.read(&mut bytes).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&bytes[..n]);
                    assert!(request.len() < 32 * 1024);
                }
                tx.send(index).unwrap();
                if index == cancelled_boundary {
                    gate.notified().await;
                }
                let _ = stream.write_all(&reply).await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        }));
        let mut client = KuwoClient::test_client();
        client.web_test_origin = Some(origin);
        client.native_test_response_key = Some(KEY);
        client.http = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .build()
            .unwrap();
        let task = tokio::spawn(async move {
            client
                .login_native_password(&request(), &device::fixture_device())
                .await
        });
        for expected in 0..=cancelled_boundary {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), seen.recv())
                    .await
                    .unwrap(),
                Some(expected)
            );
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.notify_one();
        (&mut server.0).await.unwrap();
    }
}
