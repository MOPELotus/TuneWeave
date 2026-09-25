use super::*;
use crate::{KugouConfig, signing::android_signature};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const KEY: &str = "abcd0123456789abcdef0123456789abcdef";

fn envelope(data: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"status":1,"error_code":0,"data":data})).unwrap()
}
fn created() -> Value {
    json!({"qrcode":KEY,"qrcode_img":"https://untrusted.invalid/never-fetch"})
}
fn authorized() -> Value {
    json!({"status":4,"userid":"123456789","token":"synthetic-qr-token"})
}
fn response(data: Value) -> String {
    response_bytes(envelope(data), "application/json; charset=utf-8")
}
fn response_bytes(body: Vec<u8>, content_type: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        String::from_utf8(body).unwrap()
    )
}

async fn server(
    frames: Vec<(String, Duration)>,
) -> (
    KugouClient,
    tokio::task::JoinHandle<Vec<String>>,
    tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (frame, delay) in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                assert!(request.len() < 65536);
                if request.windows(4).any(|v| v == b"\r\n\r\n") {
                    break;
                }
            }
            requests.push(String::from_utf8(request).unwrap());
            let _ = tx.send(());
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let _ = socket.write_all(frame.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
        requests
    });
    let mut client = KugouClient::new(&KugouConfig::default()).unwrap();
    client.http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    client.login_test_origin = Some(origin);
    (client, task, rx)
}
fn frames(data: Vec<Value>) -> Vec<(String, Duration)> {
    data.into_iter()
        .map(|v| (response(v), Duration::ZERO))
        .collect()
}
async fn allow_poll(session: &KugouQrSession) {
    session.state.lock().await.next_poll = Instant::now();
}
fn params(request: &str, path: &str) -> BTreeMap<String, String> {
    let line = request.lines().next().unwrap();
    assert!(line.starts_with(&format!("GET {path}?")));
    let target = line.split_whitespace().nth(1).unwrap();
    let u = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    let list = u.query_pairs().collect::<Vec<_>>();
    let map = list
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(list.len(), map.len());
    for header in [
        "cookie:",
        "authorization:",
        "pacmtoken:",
        "x-real-ip:",
        "x-forwarded-for:",
    ] {
        assert!(!request.to_ascii_lowercase().contains(header));
    }
    assert!(!map.contains_key("token"));
    assert!(!map.contains_key("userid"));
    map
}

#[test]
fn client_types_have_fixed_independent_protocol_parameters() {
    let identity = KugouDevice::default().identity();
    for (kind, appid, ver, create, stamp) in [
        (KugouLoginClient::Standard, 1005, 20489, 1001, "1700000000"),
        (KugouLoginClient::Concept, 3116, 11440, 1001, "1700000000"),
        (KugouLoginClient::Web, 1014, 8131, 1014, "1700000000123"),
    ] {
        let p = kind.parameters(&identity, Duration::from_millis(1700000000123));
        assert_eq!(p["appid"], appid.to_string());
        assert_eq!(p["clientver"], ver.to_string());
        assert_eq!(p["clienttime"], stamp);
        assert_eq!(kind.create_appid(), create);
        assert_eq!(
            p["uuid"],
            if kind == KugouLoginClient::Web {
                identity.mid.as_str()
            } else {
                "-"
            }
        );
        assert_eq!(p["mid"], identity.mid);
        assert_eq!(p["dfid"], "-");
    }
}

#[test]
fn fixed_signatures_match_independent_raw_utf8_and_body_vectors() {
    let parameters = BTreeMap::from([
        ("appid", "1014".to_owned()),
        ("clienttime", "1700000000123".to_owned()),
        ("clientver", "8131".to_owned()),
        ("dfid", "-".to_owned()),
        ("mid", "123456789".to_owned()),
        ("plat", "4".to_owned()),
        ("qrcode_txt", format!("{QR_PAGE}?appid=1014&")),
        ("srcappid", "2919".to_owned()),
        ("type", "1".to_owned()),
        ("uuid", "123456789".to_owned()),
    ]);
    let body = r#"{"name":"A&B / 中文","ids":[1,2]}"#.as_bytes();
    assert_eq!(
        web_signature(&parameters, &[]),
        "647AA8BA1DBD08E13E17085E3013BB01"
    );
    assert_eq!(
        web_signature(&parameters, body),
        "5B9636232ABC532D281AC178F2610BDA"
    );
    assert_eq!(
        android_signature(&parameters, &[]),
        "77df8d98cff86a28a27e2cd0e134d189"
    );
    assert_eq!(
        android_signature(&parameters, body),
        "f674bee8aa7995def1c496a117b5bfcd"
    );
    let reformatted = br#"{ "ids": [1,2], "name": "A&B / \u4e2d\u6587" }"#;
    assert_ne!(
        web_signature(&parameters, body),
        web_signature(&parameters, reformatted)
    );
}

#[test]
fn native_password_challenge_parser_accepts_verified_image_and_browser_forms() {
    let png = b"\x89PNG\r\n\x1a\nsynthetic";
    let image = parse_native_password_challenge(
        &serde_json::to_vec(&json!({
            "status": 1,
            "data": {
                "verifykey": "synthetic-key",
                "verifycode": BASE64.encode(png),
                "serpath": ""
            }
        }))
        .unwrap(),
        0,
    )
    .unwrap();
    assert_eq!(image.code_type(), 0);
    assert_eq!(image.verify_key(), Some("synthetic-key"));
    let expected_image = format!("data:image/png;base64,{}", BASE64.encode(png));
    assert_eq!(image.image_data_url(), Some(expected_image.as_str()));
    assert!(image.browser_url().is_none());
    assert!(!format!("{image:?}").contains("synthetic-key"));

    let browser = parse_native_password_challenge(
        &serde_json::to_vec(&json!({
            "status": 1,
            "data": {
                "verifykey": "synthetic-browser-key",
                "verifycode": "",
                "serpath": "https://verify.kugou.com/challenge?id=synthetic"
            }
        }))
        .unwrap(),
        3,
    )
    .unwrap();
    assert_eq!(browser.code_type(), 3);
    assert!(browser.image_data_url().is_none());
    assert_eq!(
        browser.browser_url(),
        Some("https://verify.kugou.com/challenge?id=synthetic")
    );
    assert_eq!(browser.browser_target(), browser.browser_url());

    for target in [
        "KGCodeTX|1234567890",
        r#"KGCodeGT|{"gt":"synthetic-gt","challenge":"synthetic-challenge","success":1}"#,
    ] {
        let modern = parse_native_password_challenge(
            &serde_json::to_vec(&json!({"status":1,"data":{"serpath":target}})).unwrap(),
            3,
        )
        .unwrap();
        assert_eq!(modern.browser_target(), Some(target));
        assert!(modern.browser_url().is_none());
        assert!(modern.verify_key().is_none());
        assert!(!format!("{modern:?}").contains(target));
    }
}

#[test]
fn native_password_challenge_parser_rejects_unsafe_or_incomplete_material() {
    for data in [
        json!({"status":0,"data":null}),
        json!({"status":1,"data":null}),
        json!({"status":1,"data":{}}),
        json!({"status":1,"data":{"verifykey":"bad key","serpath":"https://verify.kugou.com/"}}),
        json!({"status":1,"data":{"verifykey":"key","verifycode":"not-base64"}}),
        json!({"status":1,"data":{"verifykey":"key","verifycode":BASE64.encode(b"not-an-image")}}),
        json!({"status":1,"data":{"serpath":"javascript:alert(1)"}}),
        json!({"status":1,"data":{"serpath":"KGCodeTX|invalid"}}),
        json!({"status":1,"data":{"serpath":"KGCodeGT|not-json"}}),
    ] {
        assert!(
            parse_native_password_challenge(&serde_json::to_vec(&data).unwrap(), 0).is_err(),
            "unexpectedly accepted {data}"
        );
    }
}

#[tokio::test]
async fn native_password_challenge_fetch_matches_official_unsigned_query() {
    let png = b"\x89PNG\r\n\x1a\nsynthetic";
    let frame = response_bytes(
        serde_json::to_vec(&json!({
            "status": 1,
            "data": {"verifykey":"synthetic-key", "verifycode":BASE64.encode(png)}
        }))
        .unwrap(),
        "application/json; charset=utf-8",
    );
    let (client, requests, _) = server(vec![(frame, Duration::ZERO)]).await;
    let challenge = client
        .fetch_native_password_challenge(KugouNativePasswordChallengeKind::Interactive)
        .await
        .unwrap();
    assert_eq!(challenge.code_type(), 3);
    assert_eq!(challenge.verify_key(), Some("synthetic-key"));
    assert!(challenge.image_data_url().is_some());
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let query = params(&requests[0], NATIVE_PASSWORD_CHALLENGE_PATH);
    assert_eq!(query.len(), 6);
    assert_eq!(query["appid"], NATIVE_APP_ID.to_string());
    assert_eq!(query["clientver"], NATIVE_CLIENT_VERSION.to_string());
    assert_eq!(query["type"], NATIVE_PASSWORD_CHALLENGE_TYPE);
    assert_eq!(query["codetype"], "3");
    assert_eq!(query["client_type"], "4");
    assert!(!query.contains_key("signature"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("user-agent: android15-1070-11083-46-0-discoverydradprotocol-wifi")
    );
}

#[test]
fn qr_parsers_reject_invalid_identity_envelopes_and_duplicate_fields() {
    assert_eq!(parse_create(&envelope(created())).unwrap(), KEY);
    for key in [
        "",
        "../../x",
        "abcd0123456789abcdef?appid=1005",
        "abcd0123456789abcdef&token=wrong",
        "abcd0123456789abc def",
    ] {
        assert!(parse_create(&envelope(json!({"qrcode":key}))).is_err());
    }
    assert!(parse_create(&envelope(json!({"qrcode":"x".repeat(129)}))).is_err());
    assert_eq!(
        parse_create(&envelope(json!({"qrcode":"Case_Sensitive-Key-0123456789"}))).unwrap(),
        "Case_Sensitive-Key-0123456789"
    );
    for bytes in [
        br#"{}"#.as_slice(),
        br#"{"status":1,"error_code":0,"data":[]}"#,
        br#"{"status":1,"error_code":0,"data":null}"#,
        br#"{"status":1,"error_code":0,"data":{"qrcode":null}}"#,
        br#"{"status":1,"status":0,"error_code":0,"data":{}}"#,
        br#"{"status":1,"error_code":0,"data":{"qrcode":"a","qrcode":"b"}}"#,
    ] {
        assert!(parse_create(bytes).is_err());
    }
    let rejected = json!({"status":0,"error_code":9876,"data":null,"msg":"secret-token"});
    let e = parse_create(&serde_json::to_vec(&rejected).unwrap()).unwrap_err();
    assert_eq!(e.details["platform_code"], 9876);
    assert!(!format!("{e:?}").contains("secret-token"));
}

#[test]
fn authorization_is_typed_unverified_material_and_never_a_profile() {
    let device = KugouDevice::default().identity();
    for kind in [
        KugouLoginClient::Standard,
        KugouLoginClient::Concept,
        KugouLoginClient::Web,
    ] {
        let result = parse_poll(&envelope(authorized()), KEY, kind, &device).unwrap();
        assert!(!format!("{result:?}").contains("synthetic-qr-token"));
        let KugouQrPoll::AuthorizationReceived(auth) = result else {
            panic!("expected raw authorization");
        };
        assert_eq!(auth.user_id(), "123456789");
        assert_eq!(auth.token(), "synthetic-qr-token");
        assert_eq!(auth.client_kind(), kind);
        assert_eq!(auth.device_guid(), device.guid);
        assert_eq!(auth.device_mid(), device.mid);
        assert!(auth.device_dfid().is_none());
        assert!(!format!("{auth:?}").contains("123456789"));
    }
    for state in [0, 1, 2] {
        assert!(
            parse_poll(
                &envelope(json!({"status":state})),
                KEY,
                KugouLoginClient::Standard,
                &device
            )
            .is_ok()
        );
        assert!(
            parse_poll(
                &envelope(json!({"status":state,"token":"unexpected-token"})),
                KEY,
                KugouLoginClient::Standard,
                &device
            )
            .is_err()
        );
    }
    let mut malformed = Vec::new();
    for uid in [
        Value::Null,
        json!(0),
        json!(-1),
        json!(1.5),
        json!("01"),
        json!("0"),
        json!("+1"),
        json!("1e3"),
        json!("18446744073709551616"),
        json!(""),
    ] {
        let mut v = authorized();
        v["userid"] = uid;
        malformed.push(v);
    }
    for token in [
        Value::Null,
        json!(""),
        json!("null"),
        json!("undefined"),
        json!("x\ny"),
        json!("x y"),
        json!("密钥"),
        json!("x".repeat(16385)),
        json!(123),
    ] {
        let mut v = authorized();
        v["token"] = token;
        malformed.push(v);
    }
    for extra in [
        json!({"appid":1014}),
        json!({"qrcode":"another-transaction"}),
        json!({"status":3}),
        json!({"status":255}),
    ] {
        let mut v = authorized();
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        malformed.push(v);
    }
    for v in malformed {
        assert!(parse_poll(&envelope(v), KEY, KugouLoginClient::Standard, &device).is_err());
    }
    assert!(parse_poll(br#"{"status":1,"error_code":0,"data":{"status":4,"userid":"1","token":"a","token":"b"}}"#,KEY,KugouLoginClient::Standard,&device).is_err());
}

#[tokio::test]
async fn qr_requests_keep_client_and_device_across_encoded_fixed_endpoints() {
    for kind in [
        KugouLoginClient::Standard,
        KugouLoginClient::Concept,
        KugouLoginClient::Web,
    ] {
        let (client, requests, _) = server(frames(vec![
            created(),
            json!({"status":1}),
            json!({"status":2,"nickname":"unverified"}),
            authorized(),
        ]))
        .await;
        let session = client.create_login_qr(kind).await.unwrap();
        assert_eq!(
            session.url(),
            format!("{QR_PAGE}?appid={}&qrcode={KEY}", kind.appid())
        );
        assert!(session.expires_in() <= LOCAL_LIFETIME);
        assert!(session.expires_in() > Duration::from_secs(290));
        assert!(!format!("{session:?}").contains(KEY));
        assert!(matches!(
            session.poll().await.unwrap(),
            KugouQrPoll::WaitingForScan
        ));
        assert_eq!(
            session.poll().await.unwrap_err().code,
            ErrorCode::RateLimited
        );
        allow_poll(&session).await;
        assert!(matches!(
            session.poll().await.unwrap(),
            KugouQrPoll::WaitingForConfirmation
        ));
        allow_poll(&session).await;
        assert!(matches!(
            session.poll().await.unwrap(),
            KugouQrPoll::AuthorizationReceived(_)
        ));
        assert_eq!(
            session.clone().poll().await.unwrap_err().code,
            ErrorCode::Conflict
        );
        let req = requests.await.unwrap();
        assert_eq!(req.len(), 4);
        let create = params(&req[0], CREATE_PATH);
        assert_eq!(create["appid"], kind.create_appid().to_string());
        assert_eq!(
            create["qrcode_txt"],
            format!("{QR_PAGE}?appid={}&", kind.appid())
        );
        assert_eq!(create["type"], "1");
        for (i, request) in req.iter().enumerate() {
            let p = params(request, if i == 0 { CREATE_PATH } else { POLL_PATH });
            assert_eq!(p.len(), if i == 0 { 11 } else { 10 });
            assert_eq!(p["mid"], create["mid"]);
            assert_eq!(p["uuid"], create["uuid"]);
            assert_eq!(p["dfid"], "-");
            assert_eq!(p["clientver"], kind.clientver().to_string());
            assert_eq!(p["srcappid"], "2919");
            assert_eq!(p["plat"], "4");
            let unsigned = p
                .iter()
                .filter(|(k, _)| k.as_str() != "signature")
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect();
            assert_eq!(p["signature"], web_signature(&unsigned, &[]));
            if i > 0 {
                assert_eq!(p["appid"], kind.appid().to_string());
                assert_eq!(p["qrcode"], KEY);
            }
        }
    }
}

#[tokio::test]
async fn isolated_qr_devices_do_not_modify_persistent_anonymous_identity() {
    let path =
        std::env::temp_dir().join(format!("tuneweave-kugou-qr-{}.json", rand::random::<u64>()));
    let (mut client, requests, _) = server(frames(vec![created(), created()])).await;
    let mut public_client = KugouClient::new(&KugouConfig {
        device_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    public_client.login_test_origin = client.login_test_origin;
    public_client.http = client.http;
    client = public_client;
    let before = std::fs::read(&path).unwrap();
    let anonymous = serde_json::from_slice::<Value>(&before).unwrap();
    let a = client
        .create_login_qr(KugouLoginClient::Standard)
        .await
        .unwrap();
    let b = client
        .clone()
        .create_login_qr(KugouLoginClient::Concept)
        .await
        .unwrap();
    assert_ne!(a.identity.mid, b.identity.mid);
    assert_ne!(a.identity.mid, anonymous["mid"]);
    assert_ne!(b.identity.mid, anonymous["mid"]);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut store = crate::device::DeviceStore::open(Some(path.clone())).unwrap();
    let rotated = store.rotate().unwrap();
    assert_ne!(a.identity.mid, rotated.mid);
    assert_ne!(b.identity.mid, rotated.mid);
    a.cancel();
    assert_eq!(
        a.clone().poll().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    b.cancel();
    assert_eq!(requests.await.unwrap().len(), 2);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn cancellation_and_expiration_discard_late_authorization() {
    for cancel in [true, false] {
        let (client, requests, mut received) = server(vec![
            (response(created()), Duration::ZERO),
            (response(authorized()), Duration::from_millis(100)),
        ])
        .await;
        let mut session = client
            .create_login_qr(KugouLoginClient::Standard)
            .await
            .unwrap();
        received.recv().await.unwrap();
        if !cancel {
            session.deadline = Instant::now() + Duration::from_millis(50);
        }
        let cloned = session.clone();
        let task = tokio::spawn(async move { cloned.poll().await });
        received.recv().await.unwrap();
        assert_eq!(
            session.poll().await.unwrap_err().code,
            ErrorCode::RateLimited
        );
        if cancel {
            session.cancel();
            assert_eq!(session.poll().await.unwrap_err().code, ErrorCode::Conflict);
        }
        let outcome = task.await.unwrap();
        if cancel {
            assert_eq!(outcome.unwrap_err().code, ErrorCode::Conflict);
        } else {
            assert!(matches!(outcome.unwrap(), KugouQrPoll::Expired));
            assert!(matches!(
                session.poll().await.unwrap(),
                KugouQrPoll::Expired
            ));
        }
        assert_eq!(requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn local_and_upstream_expiry_are_terminal_without_more_requests() {
    for upstream in [true, false] {
        let data = if upstream {
            vec![created(), json!({"status":0})]
        } else {
            vec![created()]
        };
        let (client, requests, _) = server(frames(data)).await;
        let mut session = client
            .create_login_qr(KugouLoginClient::Standard)
            .await
            .unwrap();
        if !upstream {
            session.deadline = Instant::now();
        }
        for _ in 0..2 {
            assert!(matches!(
                session.poll().await.unwrap(),
                KugouQrPoll::Expired
            ));
        }
        assert_eq!(requests.await.unwrap().len(), if upstream { 2 } else { 1 });
    }
}

#[tokio::test]
async fn transport_rejects_redirects_bad_types_oversized_and_business_failures() {
    let body = "x".repeat(RESPONSE_LIMIT + 1);
    let cases = vec![
        ("HTTP/1.1 302 Found\r\nLocation: https://untrusted.invalid/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),ErrorCode::UpstreamError),
        ("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),ErrorCode::RateLimited),
        ("HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),ErrorCode::UpstreamError),
        (response_bytes(envelope(created()),"text/html"),ErrorCode::UpstreamError),
        (response_bytes(body.as_bytes().to_vec(),"application/json"),ErrorCode::UpstreamError),
        (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len()),ErrorCode::UpstreamError),
        (response_bytes(br#"{"status":0,"error_code":100,"msg":"secret-token"}"#.to_vec(),"application/json"),ErrorCode::UpstreamError),
    ];
    for (frame, code) in cases {
        let (client, requests, _) = server(vec![(frame, Duration::ZERO)]).await;
        let error = client
            .create_login_qr(KugouLoginClient::Standard)
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(!format!("{error:?}").contains("secret-token"));
        assert!(!format!("{error:?}").contains("untrusted.invalid"));
        assert!(!error.retryable);
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn transport_timeout_is_redacted_and_does_not_restart_the_transaction() {
    let (mut client, requests, _) =
        server(vec![(response(created()), Duration::from_millis(100))]).await;
    client.http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(25))
        .build()
        .unwrap();
    let error = client
        .create_login_qr(KugouLoginClient::Standard)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamTimeout);
    assert!(!error.retryable);
    assert!(!format!("{error:?}").contains("http"));
    assert_eq!(requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn upstream_rate_limit_extends_shared_poll_cooldown_without_automatic_retry() {
    let limited = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 45\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let (client, requests, _) = server(vec![
        (response(created()), Duration::ZERO),
        (limited.to_owned(), Duration::ZERO),
    ])
    .await;
    let session = client.create_login_qr(KugouLoginClient::Web).await.unwrap();
    let error = session.poll().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::RateLimited);
    assert_eq!(error.details["retry_after_secs"], 45);
    let again = session.clone().poll().await.unwrap_err();
    assert_eq!(again.code, ErrorCode::RateLimited);
    assert!(again.details["retry_after_secs"].as_u64().unwrap() >= 44);
    session.cancel();
    assert_eq!(requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn failed_poll_preserves_the_original_device_and_key_for_an_explicit_retry() {
    for failed in [
        "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        response(json!({"status":4,"userid":0,"token":"unverified-secret"})),
    ] {
        let (client, requests, _) = server(vec![
            (response(created()), Duration::ZERO),
            (failed, Duration::ZERO),
            (response(json!({"status":1})), Duration::ZERO),
        ])
        .await;
        let session = client
            .create_login_qr(KugouLoginClient::Concept)
            .await
            .unwrap();
        let e = session.poll().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(!format!("{e:?}").contains("unverified-secret"));
        assert_eq!(
            session.poll().await.unwrap_err().code,
            ErrorCode::RateLimited
        );
        allow_poll(&session).await;
        assert!(matches!(
            session.poll().await.unwrap(),
            KugouQrPoll::WaitingForScan
        ));
        session.cancel();
        let req = requests.await.unwrap();
        assert_eq!(req.len(), 3);
        let before = params(&req[1], POLL_PATH);
        let after = params(&req[2], POLL_PATH);
        for key in ["appid", "clientver", "qrcode", "mid", "uuid", "dfid"] {
            assert_eq!(before[key], after[key]);
        }
    }
}

#[tokio::test]
#[ignore = "anonymous official QR create/wait probe; no scanning or account validation"]
async fn official_qr_create_and_wait_for_each_fixed_client() {
    let client = KugouClient::new(&KugouConfig::default()).unwrap();
    for kind in [
        KugouLoginClient::Standard,
        KugouLoginClient::Concept,
        KugouLoginClient::Web,
    ] {
        let session = client.create_login_qr(kind).await.unwrap();
        let result = session.poll().await.unwrap();
        assert!(matches!(result, KugouQrPoll::WaitingForScan));
        session.cancel();
    }
}
