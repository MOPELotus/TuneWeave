use super::*;
use crate::client::passport::tests::{login, response, server};
use crate::passport::rsa::tests::{decrypt, key_json};

fn key(index: usize, cookies: &str) -> String {
    response(key_json(index), cookies)
}

#[tokio::test]
async fn sms_protocol_binds_fresh_keys_and_incremental_cookies_to_each_step() {
    let (client, requests) = server(vec![
        key(0, "Set-Cookie: mgnd_session_id=first; Path=/\r\n"),
        key(1, "Set-Cookie: mgnd_session_id=second; Path=/\r\n"),
        response(
            json!({"status":2000,"result":{"captchaId":"not-a-form-field"}}),
            "Set-Cookie: mgnd_session_last_access=sent; Path=/\r\n",
        ),
        key(1, "Set-Cookie: mgnd_session_id=third; Path=/\r\n"),
        login(),
    ])
    .await;
    let mut cookies = PassportCookies::default();
    client
        .send_sms("13800138000", "", &mut cookies, &|| Ok(()))
        .await
        .unwrap();
    let token = client
        .authenticate_sms("13800138000", "123456", "", &mut cookies, &|| Ok(()))
        .await
        .unwrap();
    assert_eq!(token, "intermediate&token?#");
    let requests = requests.await.unwrap();
    assert!(requests[0].starts_with("POST /password/publickey "));
    assert!(!requests[0].contains("cookie:"));
    assert!(requests[1].contains("cookie: mgnd_session_id=first\r\n"));
    let query = requests[2]
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .split_once('?')
        .unwrap()
        .1;
    let fields = url::form_urlencoded::parse(query.as_bytes()).collect::<BTreeMap<_, _>>();
    assert_eq!(fields.len(), 7);
    assert_eq!(decrypt(0, &fields["msisdn"]), b"13800138000");
    assert_eq!(fields["imgcodeType"], "2");
    assert_eq!(fields["sourceID"], "220029");
    assert!(requests[2].starts_with("GET /login/dynamicpassword?"));
    assert!(requests[2].contains("cookie: mgnd_session_id=second\r\n"));
    assert!(requests[3].contains("mgnd_session_last_access=sent"));
    assert!(requests[4].starts_with("POST /authn/dynamicpassword HTTP/1.1"));
    assert!(
        requests[4].contains("cookie: mgnd_session_id=third; mgnd_session_last_access=sent\r\n")
    );
    let fields =
        url::form_urlencoded::parse(requests[4].split_once("\r\n\r\n").unwrap().1.as_bytes())
            .collect::<BTreeMap<_, _>>();
    assert_eq!(fields.len(), 11);
    assert_eq!(decrypt(1, &fields["msisdn"]), b"13800138000");
    assert_eq!(decrypt(1, &fields["dynamicPassword"]), b"123456");
    assert_eq!(fields["securityCode"], "");
    assert!(!fields.contains_key("captchaId"));
    assert_eq!(fields["imgcodeType"], "2");
    assert!(
        requests
            .iter()
            .all(|v| !v.contains("msisdn=13800138000&") && !v.contains("dynamicPassword=123456&"))
    );
}

#[tokio::test]
async fn rejected_sms_code_retains_its_response_cookie_for_the_next_fresh_key() {
    let (client, requests) = server(vec![
        key(0, "Set-Cookie: mgnd_session_id=before; Path=/\r\n"),
        response(
            json!({"status":"4005","message":"do-not-echo"}),
            "Set-Cookie: mgnd_session_id=after; Path=/\r\n",
        ),
        key(1, ""),
        login(),
    ])
    .await;
    let mut cookies = PassportCookies::default();
    let error = client
        .authenticate_sms("13800138000", "000000", "", &mut cookies, &|| Ok(()))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    assert_eq!(error.details["platform_code"], "4005");
    assert!(!error.to_string().contains("do-not-echo"));
    client
        .authenticate_sms("13800138000", "1234", "", &mut cookies, &|| Ok(()))
        .await
        .unwrap();
    let requests = requests.await.unwrap();
    assert!(requests[2].contains("cookie: mgnd_session_id=after\r\n"));
    assert!(requests[3].contains("cookie: mgnd_session_id=after\r\n"));
    let fields =
        url::form_urlencoded::parse(requests[3].split_once("\r\n\r\n").unwrap().1.as_bytes())
            .collect::<BTreeMap<_, _>>();
    assert_eq!(decrypt(1, &fields["dynamicPassword"]), b"1234");
}
