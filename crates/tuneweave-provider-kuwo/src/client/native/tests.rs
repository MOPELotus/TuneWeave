use super::*;
use crate::client::catalog::tests::{json_response, response};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
};

const KEY: [u8; 8] = *b"17894932";
const OLD: &str = "session-42&+%";
const NEW: &str = "refreshed-session-42";

#[cfg(debug_assertions)]
#[test]
fn native_response_diagnostic_reports_playlist_kinds_without_values() {
    let summary = diagnostic_response_shape(
        br#"{"errcode":0,"plist":[{"type":"GENERAL","id":"private-id","title":"private title"},{"type":"MYFAVORITE","id":"other-private-id"}],"token":"private-token"}"#,
    )
    .to_string();

    assert!(summary.contains("\"GENERAL\":1"));
    assert!(summary.contains("\"MYFAVORITE\":1"));
    assert!(summary.contains("\"plist_count\":2"));
    assert!(!summary.contains("private"));
}

fn input() -> KuwoNativeSessionInput {
    KuwoNativeSessionInput::new("42", OLD, "123456789", "device-user-123").unwrap()
}
fn accepted() -> serde_json::Value {
    json!({"result":"succ","sid":NEW,"userInfo":{"uid":42,"nickName":"Listener","password":"never-export","other":{"token":"never-export"}}})
}
pub(crate) fn encrypted(value: &serde_json::Value) -> Vec<u8> {
    response(
        200,
        "application/json",
        "Set-Cookie: ignored=never-export; Path=/\r\n",
        &codec::fixture_response(&serde_json::to_vec(value).unwrap(), &KEY),
    )
}
pub(crate) struct Fixture {
    pub(crate) client: KuwoClient,
    pub(crate) seen: mpsc::UnboundedReceiver<String>,
    pub(crate) server: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
pub(crate) async fn setup(responses: Vec<Vec<u8>>) -> Fixture {
    setup_gated(responses.into_iter().map(|body| (body, None)).collect()).await
}
pub(crate) async fn setup_gated(
    responses: Vec<(Vec<u8>, Option<Arc<tokio::sync::Notify>>)>,
) -> Fixture {
    setup_gated_with_preparation(responses, Duration::from_secs(3)).await
}
pub(crate) async fn setup_gated_with_preparation(
    responses: Vec<(Vec<u8>, Option<Arc<tokio::sync::Notify>>)>,
    preparation_wait: Duration,
) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (tx, seen) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        for (index, (response, gate)) in responses.into_iter().enumerate() {
            let wait = if index == 0 {
                preparation_wait
            } else {
                Duration::from_secs(3)
            };
            let (mut stream, _) = tokio::time::timeout(wait, listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = Vec::new();
            loop {
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&request[..end]).unwrap();
                    let length = head
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
                    if request.len() == end + 4 + length {
                        break;
                    }
                    assert!(request.len() < end + 4 + length);
                }
                let mut buffer = [0; 1024];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                assert!(request.len() < 512 * 1024);
            }
            tx.send(String::from_utf8(request).unwrap()).unwrap();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let _ = stream.write_all(&response).await;
        }
    });
    let mut client = KuwoClient::test_client();
    client.http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    client.web_test_origin = Some(origin);
    client.media_http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    client.native_test_response_key = Some(KEY);
    Fixture {
        client,
        seen,
        server,
    }
}
pub(crate) async fn requests(fixture: &mut Fixture, count: usize) -> Vec<String> {
    (&mut fixture.server).await.unwrap();
    let mut result = Vec::new();
    while let Ok(value) = fixture.seen.try_recv() {
        result.push(value);
    }
    assert_eq!(result.len(), count);
    result
}

pub(crate) fn set_request_timeout(client: &mut KuwoClient, timeout: Duration) {
    client.http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .timeout(timeout)
        .build()
        .unwrap();
}

#[test]
fn unverified_input_is_strict_and_all_secret_containers_hide_their_contents() {
    for uid in ["", "0", "00", "042", "-1", "1.0", "2147483648", "42&uid=7"] {
        assert_eq!(
            KuwoNativeSessionInput::new(uid, OLD, "123", "device-user-123")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for sid in ["", "0", "null", "undefined", "white space", "secret\nline"] {
        assert!(KuwoNativeSessionInput::new("42", sid, "123", "device-user-123").is_err());
    }
    assert!(
        KuwoNativeSessionInput::new("42", &"x".repeat(4097), "123", "device-user-123").is_err()
    );
    assert!(KuwoNativeSessionInput::new("42", OLD, "", "device-user-123").is_err());
    assert!(KuwoNativeSessionInput::new("42", OLD, "device\r\n", "device-user-123").is_err());
    let input = input();
    let exchange = parse_exchange(&serde_json::to_vec(&accepted()).unwrap(), "42", &input).unwrap();
    let shown = format!("{input:?} {exchange:?}");
    for secret in [OLD, NEW, "123456789", "Listener", "never-export"] {
        assert!(!shown.contains(secret));
    }
    assert_eq!(exchange.session().device_id(), input.device_id());
    assert_eq!(exchange.nickname(), Some("Listener"));
}

#[test]
fn exchange_requires_a_consistent_success_bound_uid_and_new_session() {
    let input = input();
    let neutral_codes =
        json!({"result":"succ","status":"0","enum":0,"sid":NEW,"userInfo":{"uid":42}});
    assert!(parse_exchange(&serde_json::to_vec(&neutral_codes).unwrap(), "42", &input).is_ok());
    let numeric_error = json!({"result":"fail","status":"1136","enum":3});
    assert_eq!(
        parse_exchange(&serde_json::to_vec(&numeric_error).unwrap(), "42", &input)
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    for user in [
        json!({"uid":42}),
        json!({"uid":"42"}),
        json!("{\"uid\":\"42\",\"nickName\":\"Listener\"}"),
    ] {
        let body = json!({"ret":"succ","sid":NEW,"userInfo":user});
        assert_eq!(
            parse_exchange(&serde_json::to_vec(&body).unwrap(), "42", &input)
                .unwrap()
                .session()
                .user_id(),
            "42"
        );
    }
    for body in [
        json!({}),
        json!({"code":0,"msg":"Login_OK"}),
        json!({"result":"succ"}),
        json!({"result":"succ","ret":"fail","sid":NEW,"userInfo":{"uid":42}}),
        json!({"result":"succ","sid":"0","userInfo":{"uid":42}}),
        json!({"result":"succ","sid":NEW,"userInfo":{"uid":43}}),
        json!({"result":"succ","sid":NEW,"userInfo":{"uid":"042"}}),
        json!({"result":"succ","sid":NEW,"userInfo":{"uid":42.0}}),
        json!({"result":"succ","sid":NEW,"userInfo":{"uid":42,"nickName":OLD}}),
        json!({"result":"succ","sid":NEW,"userInfo":{"uid":42,"nickName":"session-42%26%2b%25"}}),
        json!({"result":"succ","sid":NEW,"userInfo":{"uid":42,"nickName":"line\nbreak"}}),
        json!({"result":"succ","sid":NEW,"userInfo":{"uid":42},"status":1136}),
        json!({"result":"fail","status":1136,"enum":"3","sid":NEW}),
    ] {
        assert!(parse_exchange(&serde_json::to_vec(&body).unwrap(), "42", &input).is_err());
    }
    for raw in [
        r#"{"result":"succ","result":"fail","sid":"new-session","userInfo":{"uid":42}}"#,
        r#"{"result":"succ","sid":"new-session","userInfo":{"uid":42,"uid":43}}"#,
        r#"{"result":"succ","sid":"new-session","userInfo":"{\"uid\":42,\"uid\":43}"}"#,
    ] {
        assert!(parse_exchange(raw.as_bytes(), "42", &input).is_err());
    }
}

#[test]
fn validation_uses_explicit_business_status_and_never_http_style_success_fields() {
    assert!(parse_validation(br#"{"result":"ok"}"#).is_ok());
    assert!(parse_validation(br#"{"result":"ok","reason":""}"#).is_ok());
    for reason in ["error_user_invalid", "error_user_not_exist"] {
        let body =
            serde_json::to_vec(&json!({"result":"fail","reason":reason,"code":0,"msg":"Login_OK"}))
                .unwrap();
        assert_eq!(
            parse_validation(&body).unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
    }
    for raw in [
        r#"{"result":"fail","reason":"error_param_invalid","msg":"Login_OK","code":0}"#,
        r#"{"result":"ok","reason":"error_user_invalid"}"#,
        r#"{"result":"ok","result":"fail"}"#,
        r#"{"code":0,"msg":"Login_OK"}"#,
        r#"{"result":true}"#,
        r#"{"result":"unknown"}"#,
        "<html>OK</html>",
    ] {
        assert_eq!(
            parse_validation(raw.as_bytes()).unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
}

#[tokio::test]
async fn native_exchange_and_validation_keep_identity_devices_and_anonymous_state_separate() {
    let mut fixture = setup(vec![
        encrypted(&accepted()),
        response(200, "text/html; charset=utf-8", "", br#"{"result":"ok"}"#),
    ])
    .await;
    *fixture.client.web_session.lock().await = Some(KuwoWebSession {
        cookie_value: "public-session-marker".into(),
        refresh_after: Instant::now() + Duration::from_secs(600),
    });
    let original = input();
    let exchanged = fixture
        .client
        .exchange_native_session(&original)
        .await
        .unwrap();
    fixture
        .client
        .validate_native_session(exchanged.session())
        .await
        .unwrap();
    assert_eq!(original.session_id(), OLD);
    assert_eq!(exchanged.session().session_id(), NEW);
    assert_eq!(
        fixture
            .client
            .web_session
            .lock()
            .await
            .as_ref()
            .unwrap()
            .cookie_value,
        "public-session-marker"
    );
    let seen = requests(&mut fixture, 2).await;
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    assert!(seen[0].starts_with(&format!(
            "GET {EXCHANGE_PATH}?f=ar&q={} HTTP/1.1\r\n",
            vectors["synthetic_request"]["cipher_base64"]
                .as_str()
                .unwrap()
        )));
    let target = seen[1].split_whitespace().nth(1).unwrap();
    let parsed = Url::parse(&format!("https://loginserver.kuwo.cn{target}")).unwrap();
    let pairs: BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(pairs["type"], "new_validate_ext");
    assert_eq!(pairs["uid"], "42");
    assert_eq!(pairs["sid"], NEW);
    assert_eq!(pairs["loginSid"], NEW);
    assert_eq!(pairs["loginUid"], "42");
    for name in ["dev_id", "dev_key", "appuid"] {
        assert_eq!(pairs[name], "123456789");
    }
    assert_eq!(pairs["user"], "device-user-123");
    for request in seen {
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("\r\ncookie:"));
        assert!(!lower.contains("\r\nsecret:"));
        assert!(!request.contains("public-session-marker"));
        assert!(!request.contains("never-export"));
    }
}

#[tokio::test]
async fn exchange_errors_never_retry_or_leak_response_or_credential_material() {
    let cases = [
        (
            encrypted(&json!({"result":"fail","status":1136,"enum":"3","msg":"never-export"})),
            ErrorCode::AuthenticationRequired,
        ),
        (
            encrypted(&json!({"result":"fail","status":999,"msg":"never-export"})),
            ErrorCode::UpstreamError,
        ),
        (
            response(
                302,
                "application/json",
                "Location: https://example.test/never-export\r\n",
                b"",
            ),
            ErrorCode::UpstreamError,
        ),
        (
            response(401, "application/json", "", b"never-export"),
            ErrorCode::AuthenticationRequired,
        ),
        (
            response(403, "application/json", "", b"never-export"),
            ErrorCode::PermissionDenied,
        ),
        (
            response(
                429,
                "application/json",
                "Retry-After: 9999\r\n",
                b"never-export",
            ),
            ErrorCode::RateLimited,
        ),
        (
            response(200, "text/html", "", b"never-export"),
            ErrorCode::UpstreamError,
        ),
        (json_response(&accepted()), ErrorCode::UpstreamError),
        (
            response(
                200,
                "application/json",
                "",
                &vec![b'A'; codec::MAX_RESPONSE + 1],
            ),
            ErrorCode::UpstreamError,
        ),
    ];
    for (response, code) in cases {
        let mut fixture = setup(vec![response]).await;
        let error = fixture
            .client
            .exchange_native_session(&input())
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        for secret in [OLD, NEW, "never-export"] {
            assert!(!format!("{error:?}").contains(secret));
        }
        if code == ErrorCode::RateLimited {
            assert_eq!(error.details["retry_after_secs"], 300);
        }
        requests(&mut fixture, 1).await;
    }
}

#[tokio::test]
async fn validation_failures_are_returned_without_changing_the_supplied_session() {
    for (value, code) in [
        (
            json!({"result":"fail","reason":"error_user_invalid","kickmsg":"never-export"}),
            ErrorCode::AuthenticationRequired,
        ),
        (
            json!({"result":"fail","reason":"error_param_invalid","code":0,"msg":"Login_OK"}),
            ErrorCode::UpstreamError,
        ),
        (
            json!({"result":"ok","reason":"error_user_not_exist"}),
            ErrorCode::UpstreamError,
        ),
    ] {
        let mut fixture = setup(vec![json_response(&value)]).await;
        let session = input();
        let before = session.clone();
        let error = fixture
            .client
            .validate_native_session(&session)
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(session, before);
        assert!(!format!("{error:?}").contains("never-export"));
        requests(&mut fixture, 1).await;
    }
}

#[tokio::test]
#[ignore = "current official anonymous native protocol; never uses a real account"]
async fn live_native_zero_identity_is_denied_after_decrypting_the_official_response() {
    let client = KuwoClient::new(&KuwoConfig::default()).unwrap();
    // Deliberately bypass the public constructor only for this zero-identity probe.
    let synthetic = KuwoNativeSessionInput {
        user_id: "0".into(),
        session_id: "0".into(),
        device_id: "0".into(),
        device_user: "0".into(),
        context: None,
    };
    let error = client
        .exchange_native_session(&synthetic)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    let error = client
        .validate_native_session(&synthetic)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
}

pub(crate) fn credential_fixture(uid: &str, sid: &str) -> credential::NativeCredential {
    credential::NativeCredential::verified(
        &device::fixture_device().session_input(uid, sid).unwrap(),
    )
    .unwrap()
}
