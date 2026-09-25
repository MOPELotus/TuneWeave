use super::*;
use reqwest::Client;
use reqwest::redirect::Policy;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use url::Url;

const LOGIN_COOKIE: &str = "freshWebLoginCookieA123456789";
const UID: &str = "1234567";
const SID_RAW: &str = "webSessionSid_legacySuffix";
const SID: &str = "webSessionSid";
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3, 2, 0,
    239, 154, 19, 139, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn response(status: &str, mime: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut wire = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    wire.extend_from_slice(body);
    wire
}

fn json_response(body: &str, headers: &str) -> Vec<u8> {
    response("200 OK", "application/json", headers, body.as_bytes())
}

async fn server(
    frames: Vec<Vec<u8>>,
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
        let mut requests = Vec::new();
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
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
                    if bytes.len() >= headers_end + body_len {
                        break;
                    }
                    while bytes.len() < headers_end + body_len {
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    break;
                }
            }
            let raw = String::from_utf8(bytes).unwrap();
            tx.send(raw.clone()).ok();
            requests.push(raw);
            socket.write_all(&frame).await.unwrap();
        }
        requests
    });
    (client, task, rx)
}

fn headers(raw: &str) -> &str {
    raw.split_once("\r\n\r\n").unwrap().0
}

fn body(raw: &str) -> &str {
    raw.split_once("\r\n\r\n").unwrap().1
}

fn header<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    headers(raw)
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then(|| value.trim()))
}

#[tokio::test]
async fn web_form_login_uses_exact_official_fields_and_independently_checks_cookie_identity() {
    let home = response(
        "200 OK",
        "text/html",
        &format!(
            "Set-Cookie: {WEB_SESSION_COOKIE}={LOGIN_COOKIE}; Domain=.kuwo.cn; Path=/\r\nSet-Cookie: userid=stale-account; Path=/\r\nSet-Cookie: sid=stale-session; Path=/\r\n"
        ),
        b"<html>register</html>",
    );
    let captcha = response("200 OK", "text/html;charset=ISO-8859-1", "", PNG);
    let login = json_response(
        r#"{"status":200,"msg":"成功"}"#,
        &format!(
            "Set-Cookie: {WEB_SESSION_COOKIE}=freshLoginRotatedCookie123456789; Domain=.kuwo.cn; Path=/\r\nSet-Cookie: userid={UID}; Domain=.kuwo.cn; Path=/; HttpOnly\r\nSet-Cookie: sid={SID_RAW}; Domain=.kuwo.cn; Path=/; HttpOnly\r\n"
        ),
    );
    let check = json_response(r#"{"status":200}"#, "");
    let (client, task, mut seen) = server(vec![home, captcha, login, check]).await;
    let challenge = client.create_web_form_password_challenge().await.unwrap();
    let image = challenge.image().unwrap();
    assert!(image.image_data_url.starts_with("data:image/png;base64,"));
    challenge.validate_answer("A4b9").unwrap();
    let debug = format!("{challenge:?}");
    assert!(!debug.contains(LOGIN_COOKIE));
    let returned = client
        .submit_web_form_password(&challenge, "user@example.test", "p&ss=word", "A4b9")
        .await
        .unwrap();
    assert_eq!(returned.user_id, UID);
    assert_eq!(returned.session_id, SID);
    assert!(!format!("{returned:?}").contains(SID));

    for _ in 0..4 {
        seen.recv().await.unwrap();
    }
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with("GET /www/user/register HTTP"));
    assert!(header(&requests[0], "Cookie").is_none());
    assert!(requests[1].starts_with("GET /api/captcha/getCommonCodeWithKaptcha?"));
    assert_eq!(
        header(&requests[1], "Cookie"),
        Some(format!("{WEB_SESSION_COOKIE}={LOGIN_COOKIE}").as_str())
    );
    let query = Url::parse(&format!(
        "https://www.kuwo.cn{}",
        requests[1].split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    let query = query.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
    assert_eq!(query.get("type").map(String::as_str), Some("login"));
    assert!(query["key"].parse::<u32>().is_ok_and(|key| key < 999_999));

    assert!(requests[2].starts_with("POST /api/www/kuwoLogin HTTP"));
    assert_eq!(
        header(&requests[2], "Content-Type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        header(&requests[2], "Cookie"),
        Some(format!("{WEB_SESSION_COOKIE}={LOGIN_COOKIE}").as_str())
    );
    let form = url::form_urlencoded::parse(body(&requests[2]).as_bytes())
        .into_owned()
        .collect::<BTreeMap<_, _>>();
    assert_eq!(form["uname"], "user@example.test");
    assert_eq!(form["password"], "p&ss=word");
    assert_eq!(form["verifyCode"], "A4b9");
    assert_eq!(form["verifyCodeKey"], query["key"]);
    assert_eq!(form["retUrl"], WEB_LOGIN_REFERER);
    assert_eq!(form["keepLogin"], "1");
    assert!(!requests[2].contains("stale-account"));
    assert!(!requests[2].contains("stale-session"));

    assert!(requests[3].starts_with("POST /api/user/checkLogin HTTP"));
    assert_eq!(
        header(&requests[3], "Cookie"),
        Some(
            format!(
                "{WEB_SESSION_COOKIE}=freshLoginRotatedCookie123456789; userid={UID}; sid={SID_RAW}"
            )
            .as_str()
        )
    );
    let check_form = url::form_urlencoded::parse(body(&requests[3]).as_bytes())
        .into_owned()
        .collect::<BTreeMap<_, _>>();
    assert_eq!(check_form["uid"], UID);
    assert_eq!(check_form["sid"], SID);
}

#[tokio::test]
async fn rejected_web_password_never_checks_or_returns_an_account_identity() {
    let home = response(
        "200 OK",
        "text/html",
        &format!("Set-Cookie: {WEB_SESSION_COOKIE}={LOGIN_COOKIE}; Path=/\r\n"),
        b"<html>register</html>",
    );
    let captcha = response("200 OK", "image/png", "", PNG);
    let rejected = json_response(r#"{"status":401,"msg":"bad credential"}"#, "");
    let (client, task, mut seen) = server(vec![home, captcha, rejected]).await;
    let challenge = client.create_web_form_password_challenge().await.unwrap();
    let error = client
        .submit_web_form_password(&challenge, "user", "password", "1a")
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    assert!(!error.message.contains("bad credential"));
    for _ in 0..3 {
        seen.recv().await.unwrap();
    }
    assert_eq!(task.await.unwrap().len(), 3);
}
