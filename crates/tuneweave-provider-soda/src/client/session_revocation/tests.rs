use super::*;
use std::collections::BTreeMap;

fn source() -> SodaCredential {
    SodaCredential::test_credential("revocation-secret")
        .bind_user("123456")
        .unwrap()
}
fn reply(body: &str, cookie: Option<&str>) -> String {
    crate::test_http::json(body, cookie)
}

#[tokio::test]
async fn session_revocation_identity_uses_explicit_evidence_and_ignores_response_cookies() {
    let (origin, task) = crate::test_http::serve(vec![
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into(),
    ])
    .await;
    assert!(
        !SodaClient::test_client()
            .with_auth_test_origin(origin)
            .revocation_session_authenticated(&source())
            .await
            .unwrap()
    );
    assert_eq!(task.await.unwrap().len(), 1);
    for (body, cookie, expected) in [
        (r#"{"status_code":0,"my_info":{"id":"123456"}}"#, None, true),
        (
            r#"{"status_code":0,"my_info":{"id":"123456"}}"#,
            Some("sessionid_ss=; Max-Age=0"),
            true,
        ),
        (
            r#"{"status_code":0,"my_info":{"id":"123456"}}"#,
            Some("sessionid_ss=other-account"),
            true,
        ),
        (
            r#"{"status_code":1000016}"#,
            Some("sessionid_ss=new-cookie"),
            false,
        ),
        (r#"{"status_code":1000016,"my_info":null}"#, None, false),
    ] {
        let (origin, task) = crate::test_http::serve(vec![reply(body, cookie)]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        assert_eq!(
            client
                .revocation_session_authenticated(&source())
                .await
                .unwrap(),
            expected
        );
        let requests = task.await.unwrap();
        assert!(requests[0].starts_with("GET /luna/pc/me?aid=386088&"));
        let target = requests[0]
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = Url::parse(&format!("https://api.qishui.com{target}")).unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.len(), 9);
        assert_eq!(query["fp"], query["device_id"]);
        assert_eq!(query["device_id"], client.login_device().unwrap().device_id);
        assert_eq!(query["iid"], client.login_device().unwrap().install_id);
        assert_eq!(query["app_name"], "luna_pc");
        assert!(requests[0].contains("cookie: sessionid_ss=revocation-secret\r\n"));
    }
}

#[tokio::test]
async fn session_revocation_identity_rejects_conflicts_challenges_and_non_identity_responses() {
    let mut responses: Vec<String> = [
        "{}",
        "[]",
        r#"{"status_code":0}"#,
        r#"{"status_code":0,"my_info":{}}"#,
        r#"{"status_code":0,"my_info":{"id":"654321"}}"#,
        r#"{"status_code":0,"my_info":{"id":123456}}"#,
        r#"{"status_code":1000016,"my_info":{"id":"123456"}}"#,
        r#"{"status_code":0,"status_code":1000016}"#,
        r#"{"status_code":1000016,"my_info":false}"#,
        r#"{"status_code":"1000016"}"#,
        r#"{"status_code":5}"#,
    ]
    .into_iter()
    .map(|body| reply(body, None))
    .collect();
    responses.push(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/\r\nContent-Length: 0\r\n\r\n".into(),
    );
    responses.push("HTTP/1.1 401 Unauthorized\r\nbdturing-verify: secret-challenge\r\nContent-Length: 0\r\n\r\n".into());
    responses
        .push(reply(r#"{"status_code":1000016}"#, None).replace("application/json", "text/html"));
    for response in responses {
        let (origin, task) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let error = client
            .revocation_session_authenticated(&source())
            .await
            .unwrap_err();
        assert_ne!(error.code, ErrorCode::AuthenticationRequired);
        assert!(!format!("{error:?}").contains("secret"));
        assert_eq!(task.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn session_revocation_identity_bounds_declared_and_streamed_body() {
    let huge = format!(
        r#"{{"status_code":1000016,"padding":"{}"}}"#,
        "a".repeat(MAX_BYTES)
    );
    for response in [
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            MAX_BYTES + 1
        ),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:X}\r\n{}\r\n0\r\n\r\n",
            huge.len(),
            huge
        ),
    ] {
        let (origin, task) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        assert!(
            client
                .revocation_session_authenticated(&source())
                .await
                .is_err()
        );
        let _ = task.await;
    }
}

#[tokio::test]
async fn session_revocation_request_matches_official_device_cookie_and_pre_send_guard() {
    let client = SodaClient::test_client();
    let device = client.login_device().unwrap();
    let (origin, task) = crate::test_http::serve(vec![reply(
        "not an acknowledgement",
        Some("sessionid_ss=; Max-Age=0"),
    )])
    .await;
    let client = client.with_auth_test_origin(origin.clone());
    client
        .send_session_revocation(&source(), || Ok(()))
        .await
        .unwrap();
    let requests = task.await.unwrap();
    let target = requests[0]
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = origin.join(target).unwrap();
    assert_eq!(url.path(), REVOKE_PATH);
    assert!(requests[0].starts_with("GET "));
    assert!(requests[0].contains("cookie: sessionid_ss=revocation-secret\r\n"));
    let query = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(query.len(), 4);
    assert_eq!(query["need_redirect"], "0");
    assert_eq!(query["aid"], "386088");
    assert_eq!(query["device_id"], device.device_id);
    assert_eq!(query["fp"], device.device_id);
    let error = client
        .send_session_revocation(&source(), || {
            Err(TuneWeaveError::new(ErrorCode::Conflict, "stale source"))
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(!format!("{client:?}").contains("revocation-secret"));
}

#[tokio::test]
async fn session_revocation_unbound_input_never_initializes_device_or_sends_request() {
    let path = std::env::temp_dir().join(format!(
        "soda-revoke-unbound-{}.json",
        rand::random::<u64>()
    ));
    let client = SodaClient::new(&SodaConfig {
        device_path: Some(path.clone()),
        proxy_url: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    })
    .unwrap();
    let source = SodaCredential::test_credential("unbound-secret");
    assert!(
        client
            .revocation_session_authenticated(&source)
            .await
            .is_err()
    );
    assert!(
        client
            .send_session_revocation(&source, || panic!("unbound input reached send"))
            .await
            .is_err()
    );
    assert!(!path.exists());
}
