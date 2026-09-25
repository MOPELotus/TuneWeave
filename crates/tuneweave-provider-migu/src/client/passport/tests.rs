use super::*;
use crate::passport::rsa::tests::{decrypt, key_json};

pub(crate) fn response(body: serde_json::Value, headers: &str) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
        body.len()
    )
}
pub(crate) fn key(index: usize) -> String {
    response(
        key_json(index),
        "Set-Cookie: mgnd_session_id=passport-only; Domain=.migu.cn; Path=/; HttpOnly; Secure\r\nSet-Cookie: unwanted=discard; Path=/\r\n",
    )
}
pub(crate) fn login() -> String {
    response(
        json!({"status":2000,"result":{"token":"intermediate&token?#","redirectURL":"https://untrusted.invalid/?do-not-follow"}}),
        "",
    )
}
pub(crate) fn exchange(uid: &str) -> String {
    response(
        json!({"code":"000000","data":{"userId":uid,"usessionId":"discard-usession","msisdn":"discard-phone"}}),
        "pacmtoken: music-token\r\n",
    )
}
pub(crate) fn request(account: &str) -> PasswordLoginRequest {
    PasswordLoginRequest {
        backend: Default::default(),
        account: account.into(),
        principal_type: tuneweave_core::PrincipalType::Username,
        principal: "test-listener".into(),
        password: "密码😀-keep-private".into(),
        password_format: tuneweave_core::PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    }
}
pub(crate) async fn server(
    responses: Vec<String>,
) -> (MiguClient, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut data = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
                assert!(data.len() < 65536);
                if let Some(end) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&data[..end]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(data).unwrap());
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        requests
    });
    (
        MiguClient::test_client().with_catalog_test_origin(origin),
        handle,
    )
}

#[tokio::test]
async fn password_protocol_uses_fresh_keys_encoded_forms_and_scoped_exchange_material() {
    let (client, requests) = server(vec![
        key(0),
        login(),
        exchange("111"),
        key(1),
        login(),
        exchange("111"),
    ])
    .await;
    for _ in 0..2 {
        assert_eq!(
            client
                .password_music_session(&request("default"))
                .await
                .unwrap(),
            ("111".into(), "music-token".into())
        );
    }
    let requests = requests.await.unwrap();
    for index in 0..2 {
        let flow = &requests[index * 3..index * 3 + 3];
        assert!(flow[0].starts_with("POST /password/publickey HTTP/1.1"));
        assert!(flow[1].starts_with("POST /authn HTTP/1.1"));
        assert!(flow[1].contains("cookie: mgnd_session_id=passport-only\r\n"));
        assert!(!flow[1].contains("unwanted"));
        let body = flow[1].split_once("\r\n\r\n").unwrap().1;
        let fields = url::form_urlencoded::parse(body.as_bytes()).collect::<BTreeMap<_, _>>();
        assert_eq!(decrypt(index, &fields["loginID"]), b"test-listener");
        assert_eq!(
            decrypt(index, &fields["enpassword"]),
            [
                vec![
                    0xe5, 0xaf, 0x86, 0xe7, 0xa0, 0x81, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80
                ],
                b"-keep-private".to_vec()
            ]
            .concat()
        );
        assert_eq!(fields["sourceID"], "220029");
        assert_eq!(fields["imgcodeType"], "1");
        assert_eq!(fields["isAsync"], "true");
        assert_eq!(fields["fingerPrint"], "");
        assert_eq!(fields["fingerPrintDetail"], "");
        assert!(!body.contains("test-listener"));
        assert!(!body.contains("keep-private"));
        assert!(!flow[2].contains("cookie:"));
        assert!(!flow[2].contains("pacmtoken:"));
        let uri = flow[2].split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("https://c.musicapp.migu.cn{uri}")).unwrap();
        assert_eq!(url.path(), "/user/h5/token-validate/v3.0");
        assert!(url.fragment().is_none());
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.len(), 4);
        assert_eq!(query["token"], "intermediate&token?#");
        assert_eq!(query["activityId"], "MUSIC-WWW");
        assert!(flow[2].lines().any(|line| line.starts_with("deviceid: ")
            && line.trim_start_matches("deviceid: ").len() == 36));
    }
    let device = |r: &String| {
        r.lines()
            .find(|l| l.starts_with("deviceid:"))
            .unwrap()
            .to_owned()
    };
    assert_eq!(device(&requests[2]), device(&requests[5]));
}

#[tokio::test]
async fn invalid_passport_stages_stop_before_credentials_or_followup_requests() {
    for (responses,code) in [
        (vec![response(json!({"status":2000,"result":{"modulus":"3","publicExponent":"10001"}}),"")],ErrorCode::UpstreamError),
        (vec![key(0),response(json!({"status":4002,"message":"secret phone and password"}),"")],ErrorCode::AuthenticationRequired),
        (vec![key(0),response(json!({"status":4016}),"")],ErrorCode::RateLimited),
        (vec![key(0),response(json!({"status":4001}),"")],ErrorCode::UpstreamError),
        (vec![key(0),response(json!({"status":2000,"result":{"token":""}}),"")],ErrorCode::UpstreamError),
        (vec![key(0),login(),response(json!({"code":"000000","data":{}}),"pacmtoken: invalid\r\n")],ErrorCode::UpstreamError),
        (vec![key(0),login(),response(json!({"code":"000000","data":{"userId":"111"}}),"Set-Cookie: pacmtoken=cookie-only; Path=/\r\n")],ErrorCode::UpstreamError),
        (vec![key(0),login(),response(json!({"code":"000000","data":{"userId":"111"}}),"pacmtoken: a\r\nSet-Cookie: pacmtoken=b; Path=/\r\n")],ErrorCode::UpstreamError),
        (vec!["HTTP/1.1 302 Found\r\nLocation: https://untrusted.invalid/\r\nContent-Length: 0\r\n\r\n".into()],ErrorCode::UpstreamError),
        (vec!["HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 65537\r\n\r\n".into()],ErrorCode::UpstreamError),
    ] {
        let (client,requests)=server(responses).await;let mut e=client.password_music_session(&request("default")).await.unwrap_err();assert_eq!(e.code,code);
        assert!(e.take_caller_credential_update().is_none());
        assert!(!format!("{e:?}").contains("secret phone"));
        let expected = if e.details.get("platform_code").is_some()
            || matches!(e.code, ErrorCode::AuthenticationRequired | ErrorCode::RateLimited)
        { UpstreamBusinessClass::RejectedError } else { UpstreamBusinessClass::Unavailable };
        assert_eq!(migu_upstream_classification(&Err::<(),_>(e)).0, expected);
        requests.await.unwrap();
    }
}

#[test]
fn passport_cookie_selection_rejects_ambiguity_and_excludes_other_scopes() {
    let parse = |values: &[&str]| {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(SET_COOKIE, HeaderValue::from_str(v).unwrap());
        }
        passport_cookie(&h)
    };
    assert!(
        parse(&[
            "mgnd_session_id=x",
            "mgnd_session_id=x; Path=/elsewhere",
            "mgnd_session_id=x; Domain=other.test; Path=/"
        ])
        .unwrap()
        .is_none()
    );
    assert!(
        parse(&["mgnd_session_id=x; Path=/; Max-Age=0"])
            .unwrap()
            .is_none()
    );
    let value=parse(&["mgnd_session_id=x; Path=/; Domain=.migu.cn; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT"]).unwrap().unwrap();
    assert!(value.is_sensitive());
    assert_eq!(value, "mgnd_session_id=x");
    for headers in [
        vec!["mgnd_session_id=a; Path=/", "mgnd_session_id=b; Path=/"],
        vec!["mgnd_session_id=a; Path=/; Path=/"],
        vec!["mgnd_session_id=a b; Path=/"],
        vec!["mgnd_session_id=a; Path=/; Max-Age=no"],
    ] {
        assert!(parse(&headers).is_err());
    }
}

#[tokio::test]
#[ignore = "requires official passport HTTPS; no account data or authentication is sent"]
async fn official_public_key_supports_the_login_encoder() {
    let key = MiguClient::test_client()
        .passport_request(
            Operation::Key,
            &[],
            CookieContext::Once(None),
            None,
            |body, headers| {
                let result = body.get("result").unwrap();
                let key = LoginPublicKey::parse(
                    result["modulus"].as_str().unwrap(),
                    result["publicExponent"].as_str().unwrap(),
                )?;
                let _ = passport_cookie(headers)?;
                Ok(key)
            },
        )
        .await
        .unwrap();
    let encrypted = key.encrypt("offline-test-value-never-submitted").unwrap();
    assert!(encrypted.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(encrypted.len() % 2, 0);
}
