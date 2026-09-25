use super::*;
use crate::client::{KuwoClient, WEB_SESSION_COOKIE};
use reqwest::Client;
use reqwest::redirect::Policy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::MusicProvider;
use url::Url;

const UID: &str = "1234567";
const SID: &str = "webSessionSid";
const WEB_COOKIE: &str = "webLoginCookie123456789012";
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

async fn server(frames: Vec<Vec<u8>>) -> (KuwoProvider, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let mut client = KuwoClient::test_client();
    client.set_test_http_client(
        Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
    );
    let origin = Url::parse(&format!("http://{address}/")).unwrap();
    client.set_test_origins(origin.clone(), origin);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            let headers_end;
            let body_len;
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
            requests.push(String::from_utf8(bytes).unwrap());
            socket.write_all(&frame).await.unwrap();
        }
        requests
    });
    (KuwoProvider::from_client(client), task)
}

#[tokio::test]
async fn web_password_challenge_requires_both_web_and_native_uid_sid_validation() {
    let home = response(
        "200 OK",
        "text/html",
        &format!("Set-Cookie: {WEB_SESSION_COOKIE}={WEB_COOKIE}; Domain=.kuwo.cn; Path=/\r\n"),
        b"<html>register</html>",
    );
    let captcha = response("200 OK", "text/html", "", PNG);
    let login = response(
        "200 OK",
        "application/json",
        &format!(
            "Set-Cookie: userid={UID}; Domain=.kuwo.cn; Path=/\r\nSet-Cookie: sid={SID}; Domain=.kuwo.cn; Path=/\r\n"
        ),
        r#"{"status":200,"msg":"成功"}"#.as_bytes(),
    );
    let check_web = response("200 OK", "application/json", "", br#"{"status":200}"#);
    let register_device = response(
        "200 OK",
        "application/json",
        "",
        br#"{"code":200,"success":true,"data":{"appuid":"1234567890"}}"#,
    );
    let validate_native = response("200 OK", "application/json", "", br#"{"result":"ok"}"#);
    let (provider, task) = server(vec![
        home,
        captcha,
        login,
        check_web,
        register_device,
        validate_native,
    ])
    .await;

    let request = PasswordLoginRequest {
        backend: PasswordLoginBackend::Web,
        account: "default".into(),
        principal_type: PrincipalType::Username,
        principal: "test-user".into(),
        password: "initial-password-is-not-retained".into(),
        password_format: PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    };
    let PasswordLoginProgress::Pending {
        challenge,
        verification: PasswordVerification::Image { image },
    } = provider
        .begin_password_login(&request, CredentialMode::Client)
        .await
        .unwrap()
    else {
        panic!("Web login must first return an image challenge");
    };
    assert!(image.image_data_url.starts_with("data:image/png;base64,"));
    let result = provider
        .advance_password_login(
            &challenge,
            &PasswordChallengeAction::SubmitImage {
                answer: "A1b2".into(),
                password: "correct-password".into(),
            },
        )
        .await
        .unwrap();
    let PasswordLoginProgress::Confirmed(result) = result else {
        panic!("validated Web login must return a confirmed result");
    };
    assert!(result.profile.authenticated);
    assert_eq!(result.profile.user_id.as_deref(), Some(UID));
    let credential = result.credential.expect("client mode returns a credential");
    assert_eq!(credential.kind, "kuwo_native_v1");
    assert_eq!(task.await.unwrap().len(), 6);
}

#[tokio::test]
async fn rejected_web_password_consumes_the_local_challenge_receipt() {
    let home = response(
        "200 OK",
        "text/html",
        &format!("Set-Cookie: {WEB_SESSION_COOKIE}={WEB_COOKIE}; Domain=.kuwo.cn; Path=/\r\n"),
        b"<html>register</html>",
    );
    let captcha = response("200 OK", "image/png", "", PNG);
    let rejected = response(
        "200 OK",
        "application/json",
        "",
        br#"{"status":401,"msg":"captcha rejected"}"#,
    );
    let (provider, task) = server(vec![home, captcha, rejected]).await;
    let request = PasswordLoginRequest {
        backend: PasswordLoginBackend::Web,
        account: "default".into(),
        principal_type: PrincipalType::Username,
        principal: "test-user".into(),
        password: "initial-password-not-retained".into(),
        password_format: PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    };
    let PasswordLoginProgress::Pending { challenge, .. } = provider
        .begin_password_login(&request, CredentialMode::Client)
        .await
        .unwrap()
    else {
        panic!("Web login must first return an image challenge");
    };
    let action = PasswordChallengeAction::SubmitImage {
        answer: "A1b2".into(),
        password: "resubmitted-password".into(),
    };
    let error = provider
        .advance_password_login(&challenge, &action)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    assert!(error.auth_challenge_consumed());
    assert!(!error.message.contains("captcha rejected"));
    assert!(!format!("{error:?}").contains("resubmitted-password"));
    let replay = provider
        .advance_password_login(&challenge, &action)
        .await
        .unwrap_err();
    assert_eq!(replay.code, ErrorCode::ResourceNotFound);
    assert_eq!(task.await.unwrap().len(), 3);
}

#[tokio::test]
async fn web_password_rejects_non_username_inputs_before_network() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let mut client = KuwoClient::test_client();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    client.set_test_origins(origin.clone(), origin);
    let provider = KuwoProvider::from_client(client);
    let mut request = PasswordLoginRequest {
        backend: PasswordLoginBackend::Web,
        account: "default".into(),
        principal_type: PrincipalType::Phone,
        principal: "13800000000".into(),
        password: "password".into(),
        password_format: PasswordFormat::Plain,
        country_code: Some("86".into()),
        secure_captcha: None,
    };
    let error = provider
        .begin_password_login(&request, CredentialMode::Client)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    request.principal_type = PrincipalType::Username;
    request.country_code = None;
    request.secure_captcha = Some("unused".into());
    let error = provider
        .begin_password_login(&request, CredentialMode::Client)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    request.secure_captcha = None;
    let error = provider
        .password_login_with_mode(&request, CredentialMode::Client)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    let error = provider.password_login(&request).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}
