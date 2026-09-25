use super::*;
use reqwest::{Client, redirect::Policy};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

const COOKIE_VALUE: &str = "freshWebSmsCookieA123456789";
const PHONE: &str = "13800000000";
const CODE: &str = "24680";
const UID: &str = "1234567";

fn response(status: &str, mime: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut wire = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    wire.extend_from_slice(body);
    wire
}

fn json(body: &str, headers: &str) -> Vec<u8> {
    response("200 OK", "application/json", headers, body.as_bytes())
}

async fn server(frames: Vec<Vec<u8>>) -> (KuwoClient, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let mut client = KuwoClient::test_client();
    client.http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    client.login_test_origin = Some(origin);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 2048];
            let (headers_end, body_len);
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() <= 32 * 1024);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    headers_end = end + 4;
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    body_len = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    while bytes.len() < headers_end + body_len {
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    break;
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            socket.write_all(&frame).await.unwrap();
        }
        requests
    });
    (client, task)
}

fn target(request: &str) -> &str {
    request.split_whitespace().nth(1).unwrap()
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .split_once("\r\n\r\n")?
        .0
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then(|| value.trim()))
}

fn params(request: &str) -> BTreeMap<String, String> {
    Url::parse(&format!("https://vip1.kuwo.cn{}", target(request)))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test]
async fn official_pc_web_sms_preserves_the_producer_wire_shape_and_checks_both_sessions() {
    let home = response(
        "200 OK",
        "text/html",
        &format!("Set-Cookie: {WEB_SESSION_COOKIE}={COOKIE_VALUE}; Path=/\r\n"),
        b"<html>anonymous www session</html>",
    );
    let h5_page = response(
        "200 OK",
        "text/html",
        "",
        b"<html>official PC phone login</html>",
    );
    let sent = json(
        r#"{"meta":{"code":200},"data":{"status":200,"tm":"1700000000123"}}"#,
        "",
    );
    let logged_in = json(
        r#"{"meta":{"code":200},"data":{"result":"succ","uid":"1234567","sid":"WebSmsSession_legacy"}}"#,
        "",
    );
    let checked = json(r#"{"status":200}"#, "");
    let (client, task) = server(vec![home, h5_page, sent, logged_in, checked]).await;
    let receipt = client.send_web_login_sms(PHONE).await.unwrap();
    assert!(!format!("{receipt:?}").contains(PHONE));
    assert!(!format!("{receipt:?}").contains(COOKIE_VALUE));
    let session = client.complete_web_login_sms(&receipt, CODE).await.unwrap();
    assert_eq!(session.user_id, UID);
    assert_eq!(session.session_id, "WebSmsSession");
    assert!(!format!("{session:?}").contains("WebSmsSession"));

    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests[0].starts_with("GET / HTTP"));
    assert!(requests[1].starts_with("GET /vip/added/webView/kwOutLogin/index.html HTTP"));
    assert!(header(&requests[1], "Cookie").is_none());

    assert!(requests[2].starts_with("GET /vip/manage/hanger?"));
    assert!(header(&requests[2], "Cookie").is_none());
    assert_eq!(header(&requests[2], "Origin"), Some("https://vip1.kuwo.cn"));
    let send = params(&requests[2]);
    assert_eq!(
        send.get("op").map(String::as_str),
        Some("connectToLoginSys")
    );
    assert_eq!(send.get("url").map(String::as_str), Some(WEB_SMS_SEND_URL));
    assert_eq!(send.get("f").map(String::as_str), Some("pc"));
    assert_eq!(send.get("key").map(String::as_str), Some(WEB_SMS_KEY));
    let send_param = &send["param"];
    assert_eq!(send_param.matches("devType=pc").count(), 2);
    assert!(send_param.contains("dev_id=loginPlugin&tm="));
    assert!(send_param.ends_with(&format!("&mobile={PHONE}")));
    assert!(!send_param.contains(CODE));

    assert!(requests[3].starts_with("GET /vip/manage/hanger?"));
    assert!(header(&requests[3], "Cookie").is_none());
    let login = params(&requests[3]);
    assert_eq!(
        login.get("url").map(String::as_str),
        Some(WEB_SMS_LOGIN_URL)
    );
    let login_param = &login["param"];
    assert_eq!(login_param.matches("devType=pc").count(), 2);
    assert!(login_param.contains("devType=pc&sx=15604173\n  &from=pc"));
    assert!(login_param.contains("devResolution=240\n  &version=MUSIC_9.0.9.0_BCS5"));
    assert!(login_param.ends_with(&format!("&mobile={PHONE}&code={CODE}")));

    assert!(requests[4].starts_with("POST /api/user/checkLogin HTTP"));
    let cookies = header(&requests[4], "Cookie").unwrap();
    assert!(cookies.contains(&format!("userid={UID}")));
    assert!(cookies.contains("sid=WebSmsSession_legacy"));
    let form =
        url::form_urlencoded::parse(requests[4].split_once("\r\n\r\n").unwrap().1.as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
    assert_eq!(form.get("uid").map(String::as_str), Some(UID));
    assert_eq!(form.get("sid").map(String::as_str), Some("WebSmsSession"));
}

#[test]
fn pc_web_sms_acknowledgements_fail_closed_and_only_use_documented_fallbacks() {
    assert_eq!(
        parse_pc_sms_send_ack(
            br#"{"meta":{"code":200},"data":{"status":200,"tm":"1700000000123"}}"#,
            "1700000000999"
        )
        .unwrap(),
        "1700000000123"
    );
    for fallback in [
        r#"{"meta":{"code":200},"data":{"status":200}}"#,
        r#"{"meta":{"code":200},"data":{"status":200,"tm":null}}"#,
    ] {
        assert_eq!(
            parse_pc_sms_send_ack(fallback.as_bytes(), "1700000000999").unwrap(),
            "1700000000999"
        );
    }
    for rejected in [
        r#"{"meta":{"code":403},"data":{"status":200}}"#,
        r#"{"meta":{"code":200},"data":{"status":403}}"#,
        r#"{"meta":{"code":200},"data":{"status":200,"tm":"not-a-timestamp"}}"#,
        r#"{"meta":{"code":200},"data":{"result":"succ","uid":"1234567","sid":"sid"}}"#,
    ] {
        assert!(parse_pc_sms_send_ack(rejected.as_bytes(), "1700000000999").is_err());
    }
    assert!(
        parse_pc_sms_login_ack(
            br#"{"meta":{"code":200},"data":{"result":"failed","uid":"1234567","sid":"sid"}}"#
        )
        .is_err()
    );
    let untrusted = parse_pc_sms_login_ack(
        br#"{"meta":{"code":200},"data":{"result":"succ","uid":"0","sid":"sid"}}"#,
    )
    .unwrap();
    assert!(!valid_web_uid(&untrusted.user_id));
    assert_eq!(
        native::sms::validate_code("2468").unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(native::sms::validate_code(CODE).is_ok());
}
