use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

const COOKIE_A: &str = "freshLoginCookieA123456789";
const COOKIE_B: &str = "freshLoginCookieB123456789";
const TOKEN_A: &str = "0123456789abcdef0123456789abcdef";
const TOKEN_B: &str = "abcdef0123456789abcdef0123456789";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aE4sAAAAASUVORK5CYII=";
fn image() -> String {
    format!("data:image/png;base64,{PNG}")
}
fn body(token: &str) -> serde_json::Value {
    json!({"code":200,"data":{"token":token,"img":image()}})
}
fn response(status: &str, mime: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn reply(value: serde_json::Value) -> String {
    response("200 OK", "application/json", "", &value.to_string())
}
fn home(cookie: &str) -> String {
    response(
        "200 OK",
        "text/html",
        &format!(
            "Set-Cookie: {WEB_SESSION_COOKIE}={cookie}; Domain=.kuwo.cn; Path=/\r\nSet-Cookie: sid=ignored-account-cookie; Path=/\r\n"
        ),
        "<html>public</html>",
    )
}
struct Frame {
    wire: String,
    gate: Option<oneshot::Receiver<()>>,
}
impl From<String> for Frame {
    fn from(wire: String) -> Self {
        Self { wire, gate: None }
    }
}
fn paused(wire: String) -> (Frame, oneshot::Sender<()>) {
    let (tx, rx) = oneshot::channel();
    (
        Frame {
            wire,
            gate: Some(rx),
        },
        tx,
    )
}
async fn server(
    frames: Vec<Frame>,
) -> (
    KuwoClient,
    tokio::task::JoinHandle<Vec<String>>,
    mpsc::UnboundedReceiver<String>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let mut client = KuwoClient::test_client();
    client.http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    client.login_test_origin =
        Some(Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap());
    let (tx, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut requests = vec![];
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = vec![];
            let mut buffer = [0; 2048];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 16384);
                if bytes.windows(4).any(|s| s == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8(bytes).unwrap();
            tx.send(text.clone()).ok();
            requests.push(text);
            if let Some(gate) = frame.gate {
                let _ = gate.await;
            }
            let _ = socket.write_all(frame.wire.as_bytes()).await;
        }
        requests
    });
    (client, task, rx)
}
fn query(raw: &str) -> BTreeMap<String, String> {
    Url::parse(&format!(
        "https://www.kuwo.cn{}",
        raw.split_whitespace().nth(1).unwrap()
    ))
    .unwrap()
    .query_pairs()
    .into_owned()
    .collect()
}
fn header<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    raw.lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(k, v)| k.eq_ignore_ascii_case(name).then(|| v.trim()))
}

#[tokio::test]
async fn login_images_use_fresh_isolated_sessions_and_do_not_export_platform_tokens() {
    let (client, requests, _) = server(vec![
        home(COOKIE_A).into(),
        reply(body(TOKEN_A)).into(),
        home(COOKIE_B).into(),
        reply(body(TOKEN_B)).into(),
    ])
    .await;
    *client.web_session.lock().await = Some(KuwoWebSession {
        cookie_value: "publicCacheCookie123456789".into(),
        refresh_after: Instant::now() + WEB_SESSION_TTL,
    });
    let first = client.create_login_challenge().await.unwrap();
    let second = client.create_login_challenge().await.unwrap();
    assert_eq!(first.session.cookie, COOKIE_A);
    assert_eq!(second.session.cookie, COOKIE_B);
    assert_eq!(first.captcha.as_ref().unwrap().token, TOKEN_A);
    assert_eq!(second.captcha.as_ref().unwrap().token, TOKEN_B);
    assert!(first.expires_in_secs() > 0 && first.expires_in_secs() <= 300);
    let image = first.image().unwrap();
    assert_eq!(image.answer_kind, AuthImageAnswerKind::Alphanumeric);
    let public = serde_json::to_string(&image).unwrap();
    assert!(public.contains("alphanumeric"));
    for private in [
        TOKEN_A,
        TOKEN_B,
        COOKIE_A,
        COOKIE_B,
        "ignored-account-cookie",
    ] {
        assert!(!public.contains(private));
        assert!(!format!("{first:?} {second:?}").contains(private));
    }
    assert_eq!(
        client
            .web_session
            .lock()
            .await
            .as_ref()
            .unwrap()
            .cookie_value,
        "publicCacheCookie123456789"
    );
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 4);
    for index in [0, 2] {
        assert!(requests[index].starts_with("GET / HTTP"));
        assert!(header(&requests[index], "Cookie").is_none());
        assert!(header(&requests[index], "Secret").is_none());
    }
    for (index, cookie) in [(1, COOKIE_A), (3, COOKIE_B)] {
        let raw = &requests[index];
        assert!(raw.starts_with("GET /api/common/captcha/getcode?"));
        assert_eq!(
            header(raw, "Cookie").unwrap(),
            format!("{WEB_SESSION_COOKIE}={cookie}")
        );
        assert_eq!(header(raw, "Referer"), Some(HOME_ENDPOINT));
        let q = query(raw);
        assert_eq!(q.len(), 2);
        assert_eq!(q["httpsStatus"], "1");
        assert_eq!(q["reqId"].len(), 36);
        assert_eq!(&q["reqId"][14..15], "4");
        assert!(header(raw, "Secret").is_none());
        for forbidden in [
            "ignored-account-cookie",
            "publicCacheCookie",
            TOKEN_A,
            TOKEN_B,
        ] {
            assert!(!raw.contains(forbidden));
        }
    }
    assert_ne!(query(&requests[1])["reqId"], query(&requests[3])["reqId"]);
}

#[tokio::test]
async fn refresh_reuses_only_its_login_session_and_retires_the_previous_image_on_error() {
    let replacement = reply(body(TOKEN_B)).replace(
        "Content-Type:",
        &format!(
            "Set-Cookie: {WEB_SESSION_COOKIE}=untrustedReplacement123456; Path=/\r\nContent-Type:"
        ),
    );
    let (client, requests, _) = server(vec![
        home(COOKIE_A).into(),
        reply(body(TOKEN_A)).into(),
        replacement.into(),
        response(
            "429 Too Many Requests",
            "application/json",
            "Retry-After: 7\r\n",
            "",
        )
        .into(),
        reply(body(TOKEN_A)).into(),
    ])
    .await;
    let mut challenge = client.create_login_challenge().await.unwrap();
    assert_eq!(
        client
            .refresh_login_challenge(&mut challenge)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(challenge.captcha.as_ref().unwrap().token, TOKEN_A);
    challenge.refresh_at = Instant::now();
    client
        .refresh_login_challenge(&mut challenge)
        .await
        .unwrap();
    assert_eq!(challenge.captcha.as_ref().unwrap().token, TOKEN_B);
    challenge.refresh_at = Instant::now();
    let error = client
        .refresh_login_challenge(&mut challenge)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RateLimited);
    assert_eq!(error.details["retry_after_secs"], 7);
    assert!(challenge.refresh_after_secs() >= 6);
    assert_eq!(
        client
            .refresh_login_challenge(&mut challenge)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert!(challenge.image().is_err() && challenge.validate_answer("aB3").is_err());
    challenge.refresh_at = Instant::now();
    client
        .refresh_login_challenge(&mut challenge)
        .await
        .unwrap();
    assert_eq!(challenge.captcha.as_ref().unwrap().token, TOKEN_A);
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 5);
    for raw in &requests[1..] {
        assert_eq!(
            header(raw, "Cookie").unwrap(),
            format!("{WEB_SESSION_COOKIE}={COOKIE_A}")
        );
    }
    assert!(client.web_session.lock().await.is_none());
}

#[tokio::test]
async fn challenge_expiry_refresh_budget_and_answer_validation_do_not_send_requests() {
    let (client, requests, _) =
        server(vec![home(COOKIE_A).into(), reply(body(TOKEN_A)).into()]).await;
    let mut challenge = client.create_login_challenge().await.unwrap();
    for answer in ["a", "Ab34", "aBc123"] {
        challenge.validate_answer(answer).unwrap();
    }
    for answer in ["", "1234567", "a b", "汉字", "a\n", "é", "+1"] {
        assert_eq!(
            challenge.validate_answer(answer).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    challenge.refresh_at = Instant::now();
    challenge.refreshes = MAX_REFRESHES;
    assert_eq!(
        client
            .refresh_login_challenge(&mut challenge)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    challenge.deadline = Instant::now();
    assert_eq!(challenge.expires_in_secs(), 0);
    assert_eq!(
        client
            .refresh_login_challenge(&mut challenge)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert!(challenge.image().is_err() && challenge.validate_answer("abc").is_err());
    assert_eq!(requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn cancelled_refresh_cannot_restore_an_old_image_or_replace_the_public_cache() {
    let (frame, release) = paused(reply(body(TOKEN_B)));
    let (client, requests, mut seen) = server(vec![
        home(COOKIE_A).into(),
        reply(body(TOKEN_A)).into(),
        frame,
    ])
    .await;
    let mut challenge = client.create_login_challenge().await.unwrap();
    challenge.refresh_at = Instant::now();
    seen.recv().await.unwrap();
    seen.recv().await.unwrap();
    let mut pending = Box::pin(client.refresh_login_challenge(&mut challenge));
    tokio::select! { _=seen.recv()=>{}, result=&mut pending=>panic!("unexpected result {result:?}") }
    drop(pending);
    assert!(challenge.image().is_err());
    assert!(client.web_session.lock().await.is_none());
    release.send(()).unwrap();
    assert_eq!(requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn login_http_errors_and_transport_limits_are_terminal_without_hidden_retries() {
    let cases = vec![
        (
            response("401 Unauthorized", "application/json", "", ""),
            ErrorCode::AuthenticationRequired,
        ),
        (
            response("403 Forbidden", "application/json", "", ""),
            ErrorCode::PermissionDenied,
        ),
        (
            response(
                "429 Too Many Requests",
                "application/json",
                "Retry-After: 3\r\n",
                "",
            ),
            ErrorCode::RateLimited,
        ),
        (
            response(
                "302 Found",
                "text/html",
                "Location: https://other.invalid\r\n",
                "",
            ),
            ErrorCode::UpstreamError,
        ),
        (
            response("200 OK", "application/octet-stream", "", "data"),
            ErrorCode::UpstreamError,
        ),
        (
            response(
                "200 OK",
                "application/json",
                "",
                &"x".repeat(JSON_LIMIT as usize + 1),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            reply(json!({"code":500,"msg":TOKEN_A})),
            ErrorCode::UpstreamError,
        ),
    ];
    for (wire, code) in cases {
        let (client, requests, _) = server(vec![home(COOKIE_A).into(), wire.into()]).await;
        let error = client.create_login_challenge().await.unwrap_err();
        assert_eq!(error.code, code);
        assert!(!format!("{error:?}").contains(TOKEN_A));
        assert!(client.web_session.lock().await.is_none());
        assert_eq!(requests.await.unwrap().len(), 2);
    }
    for wire in [
        response("200 OK", "application/json", "", "{}"),
        response(
            "302 Found",
            "text/html",
            "Location: https://other.invalid\r\n",
            "",
        ),
        response("200 OK", "text/html", "", "no tracking cookie"),
    ] {
        let (client, requests, _) = server(vec![wire.into()]).await;
        assert!(client.create_login_challenge().await.is_err());
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn invalid_cookie_identity_or_scope_stops_before_captcha_creation() {
    for suffix in [
        "Domain=other.invalid; Path=/",
        "Path=/private",
        "Max-Age=0",
        "Max-Age=-1",
        "Max-Age=invalid",
    ] {
        let wire = response(
            "200 OK",
            "text/html",
            &format!("Set-Cookie: {WEB_SESSION_COOKIE}={COOKIE_A}; {suffix}\r\n"),
            "html",
        );
        let (client, requests, _) = server(vec![wire.into()]).await;
        assert!(client.create_login_challenge().await.is_err());
        assert_eq!(requests.await.unwrap().len(), 1);
    }
    let duplicate = home(COOKIE_A).replace(
        "Content-Length:",
        &format!("Set-Cookie: {WEB_SESSION_COOKIE}={COOKIE_B}; Path=/\r\nContent-Length:"),
    );
    let (client, requests, _) = server(vec![duplicate.into()]).await;
    assert!(client.create_login_challenge().await.is_err());
    assert_eq!(requests.await.unwrap().len(), 1);
}

#[test]
fn captcha_decoding_requires_explicit_success_token_and_bounded_inline_png() {
    assert!(parse_captcha(body(TOKEN_A).to_string().as_bytes()).is_ok());
    for value in [
        json!({"data":{"token":TOKEN_A,"img":image()}}),
        json!({"code":200,"data":null}),
        json!({"code":200,"data":{"token":"short","img":image()}}),
        json!({"code":200,"data":{"token":TOKEN_A,"img":"https://other.invalid/image.png"}}),
        json!({"code":200,"data":{"token":TOKEN_A,"img":"data:image/svg+xml;base64,PHN2Zz4="}}),
        json!({"code":200,"data":{"token":TOKEN_A,"img":"data:image/png;base64,YmFk"}}),
    ] {
        assert!(parse_captcha(value.to_string().as_bytes()).is_err());
    }
    let mut png = BASE64_STANDARD.decode(PNG).unwrap();
    assert!(png_envelope(&png));
    png[16..20].copy_from_slice(&0_u32.to_be_bytes());
    assert!(!png_envelope(&png));
    png[16..20].copy_from_slice(&2049_u32.to_be_bytes());
    assert!(!png_envelope(&png));
    let mut png = BASE64_STANDARD.decode(PNG).unwrap();
    png.extend_from_slice(b"trailing");
    assert!(!png_envelope(&png));
    let mut png = BASE64_STANDARD.decode(PNG).unwrap();
    png.truncate(png.len() - 1);
    assert!(!png_envelope(&png));
    assert!(!png_envelope(&vec![0; IMAGE_LIMIT + 1]));
}

#[tokio::test]
#[ignore = "contacts the official Kuwo captcha endpoint without submitting an answer or login"]
async fn live_anonymous_login_captcha_uses_the_current_official_protocol() {
    let client = KuwoClient::test_client();
    let challenge = client.create_login_challenge().await.unwrap();
    let image = challenge.image().unwrap();
    assert_eq!(image.answer_kind, AuthImageAnswerKind::Alphanumeric);
    assert!(image.image_data_url.starts_with("data:image/png;base64,"));
    assert!(challenge.expires_in_secs() > 0);
    assert!(client.web_session.lock().await.is_none());
}

#[tokio::test]
async fn late_refresh_reply_cannot_extend_the_local_challenge_deadline() {
    let (frame, release) = paused(reply(body(TOKEN_B)));
    let (client, requests, mut seen) = server(vec![
        home(COOKIE_A).into(),
        reply(body(TOKEN_A)).into(),
        frame,
    ])
    .await;
    let mut challenge = client.create_login_challenge().await.unwrap();
    challenge.refresh_at = Instant::now();
    challenge.deadline = Instant::now() + Duration::from_secs(1);
    seen.recv().await.unwrap();
    seen.recv().await.unwrap();
    let task = tokio::spawn(async move {
        let result = client.refresh_login_challenge(&mut challenge).await;
        (challenge, result)
    });
    seen.recv().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    release.send(()).unwrap();
    let (challenge, result) = task.await.unwrap();
    assert_eq!(result.unwrap_err().code, ErrorCode::ResourceNotFound);
    assert!(challenge.image().is_err() && challenge.captcha.is_none());
    assert_eq!(requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn captcha_transport_timeout_retires_the_previous_image_without_retrying() {
    let (frame, release) = paused(reply(body(TOKEN_B)));
    let (mut client, requests, mut seen) = server(vec![
        home(COOKIE_A).into(),
        reply(body(TOKEN_A)).into(),
        frame,
    ])
    .await;
    let mut challenge = client.create_login_challenge().await.unwrap();
    challenge.refresh_at = Instant::now();
    client.http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    seen.recv().await.unwrap();
    seen.recv().await.unwrap();
    let task = tokio::spawn(async move {
        let result = client.refresh_login_challenge(&mut challenge).await;
        (challenge, result)
    });
    seen.recv().await.unwrap();
    let (challenge, result) = task.await.unwrap();
    assert_eq!(result.unwrap_err().code, ErrorCode::UpstreamTimeout);
    assert!(challenge.image().is_err());
    release.send(()).unwrap();
    assert_eq!(requests.await.unwrap().len(), 3);
}
