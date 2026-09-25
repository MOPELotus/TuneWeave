use super::*;
use crate::KugouConfig;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::{PasswordFormat, PrincipalType};

const SEED: &str = "0123456789ABCDEF0123456789ABCDEF";
const UID: &str = "123456789";
const TOKEN: &str = "synthetic-native-password-token";

fn request() -> PasswordLoginRequest {
    PasswordLoginRequest {
        backend: Default::default(),
        account: "default".to_owned(),
        principal_type: PrincipalType::Phone,
        principal: "13800138000".to_owned(),
        password: "synthetic-password".to_owned(),
        password_format: PasswordFormat::Plain,
        country_code: Some("86".to_owned()),
        secure_captcha: None,
    }
}

fn success(cipher: &ExchangeCipher) -> Value {
    json!({"status":1,"error_code":0,"data":{
        "userid":UID,
        "secu_params":cipher.encrypt(&crypto::encode(&json!({"token":TOKEN})).unwrap()).unwrap(),
        "t1":"synthetic-device-token"
    }})
}

#[test]
fn native_password_body_keeps_phone_and_password_inside_encryption() {
    let request = request();
    let cipher = ExchangeCipher::random().unwrap();
    let device = KugouDevice::default().identity();
    let (mut query, bytes) = parameters(&request, &device, 1700000000123, &cipher, None).unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["username"], "138*****000");
    assert!(!String::from_utf8_lossy(&bytes).contains(&request.password));
    assert!(!String::from_utf8_lossy(&bytes).contains(&request.principal));
    assert_eq!(body["clienttime_ms"], "1700000000628");
    assert_eq!(body["key"], "d393f324c45e49f4a532d89b32ed5083");
    assert_eq!(body["t1"], "c8194f76fea010ecfe84bd132f525eda");
    assert_eq!(body["plat"], 1);
    assert_eq!(body["support_third"], "3");
    assert_eq!(body["gitversion"], "0000000");
    assert_eq!(body["busi_type"], "kid");
    assert_eq!(
        BASE64.decode(body["t3"].as_str().unwrap()).unwrap(),
        b"0,0,0,0,0,65530,0,0,0"
    );
    let secret: Value =
        serde_json::from_slice(&cipher.decrypt(body["params"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(secret["pwd"], request.password);
    assert_eq!(secret["username"], request.principal);
    assert_eq!(secret["clienttime_ms"], body["clienttime_ms"]);
    assert_eq!(query["clientver"], "20809");
    assert_eq!(query["clienttime"], "1700000000");
    assert_eq!(query["mid"], device.mid);
    assert_eq!(
        query.remove("signature").unwrap(),
        android_signature(&query, &bytes)
    );
}

#[test]
fn native_username_encoding_matches_utf16_and_native_phone_masking_rules() {
    for (input, expected) in [
        ("中文A🎵", "\\u4e2d\\u6587\\u0041\\ud83c\\udfb5"),
        ("user@example.invalid", "user@example.invalid"),
        ("🎵", "🎵"),
    ] {
        assert_eq!(native_username(input), expected);
    }
    assert!(native_phone("13800138000"));
    assert!(!native_phone("16800138000"));
    assert!(!native_phone("1380013800x"));
    let cipher = ExchangeCipher::random().unwrap();
    let device = KugouDevice::default().identity();
    for principal in ["中文A🎵", "user@example.invalid", "123456789"] {
        let mut input = request();
        input.principal = principal.to_owned();
        let (_, bytes) = parameters(&input, &device, 1700000000123, &cipher, None).unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["username"], native_username(principal));
        let secret: Value =
            serde_json::from_slice(&cipher.decrypt(body["params"].as_str().unwrap()).unwrap())
                .unwrap();
        assert!(secret.get("username").is_none());
    }
}

#[test]
fn native_success_requires_this_requests_encrypted_token_and_valid_uid() {
    let cipher = ExchangeCipher::random().unwrap();
    let device = KugouDevice::default().identity();
    let session = parse(
        &crypto::encode(&success(&cipher)).unwrap(),
        &device,
        &cipher,
    )
    .unwrap()
    .into_session()
    .unwrap();
    assert_eq!(session.user_id, UID);
    assert_eq!(session.token, TOKEN);
    assert_eq!(session.device, device);
    assert_eq!(session.t1.as_deref(), Some("synthetic-device-token"));
    for userid in [json!(0), json!(-1), json!("invalid-uid"), Value::Null] {
        let mut value = success(&cipher);
        value["data"]["userid"] = userid;
        assert!(parse(&crypto::encode(&value).unwrap(), &device, &cipher).is_err());
    }
    for data in [
        json!({"userid":UID,"token":TOKEN}),
        json!({"userid":UID,"secu_params":"00"}),
        json!({"userid":UID,"secu_params":cipher.encrypt(b"{}").unwrap()}),
        json!({"userid":UID,"secu_params":cipher.encrypt(br#"{"token":""}"#).unwrap()}),
        json!({"userid":null,"secu_params":cipher.encrypt(br#"{"token":"synthetic-token"}"#).unwrap()}),
    ] {
        let value = json!({"status":1,"data":data});
        let error = parse(&crypto::encode(&value).unwrap(), &device, &cipher)
            .and_then(NativePasswordOutcome::into_session)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains(TOKEN));
    }
    let other = ExchangeCipher::random().unwrap();
    assert!(parse(&crypto::encode(&success(&cipher)).unwrap(), &device, &other).is_err());
}

#[test]
fn native_challenges_and_rejections_never_become_sessions_or_echo_upstream_text() {
    let cipher = ExchangeCipher::random().unwrap();
    let device = KugouDevice::default().identity();
    for code in [
        30701, 30702, 30703, 30709, 20020, 20021, 30791, 30767, 30798,
    ] {
        let value = json!({"status":0,"error_code":code,"message":"private upstream detail","data":{"mobile":"13800138000"}});
        let error = parse(&crypto::encode(&value).unwrap(), &device, &cipher)
            .and_then(NativePasswordOutcome::into_session)
            .unwrap_err();
        assert_eq!(
            error.code,
            if (30701..=30703).contains(&code) {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::PermissionDenied
            }
        );
        let debug = format!("{error:?}");
        assert!(!debug.contains("private upstream detail"));
        assert!(!debug.contains("13800138000"));
    }
}

async fn server(frames: Vec<Value>) -> (KugouClient, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 65536);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let body = frame.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
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
    (client, task)
}

#[tokio::test]
async fn native_password_http_requires_profile_before_returning_a_native_credential() {
    let cipher = ExchangeCipher::for_test(SEED);
    let (mut client, task) = server(vec![
        success(&cipher),
        json!({"status":1,"error_code":0,"data":{"userid":UID,"nickname":"Synthetic account"}}),
    ])
    .await;
    client.password_test_seed = Some(SEED.into());
    let input = request();
    let result = client.login_native_password(&input).await.unwrap();
    assert_eq!(result.profile.user_id.as_deref(), Some(UID));
    let credential = KugouCredential::parse_caller(&result.credential.unwrap()).unwrap();
    assert_eq!(credential.session.token, TOKEN);
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("POST /login.user/v9/login_by_pwd?"));
    assert!(requests[1].starts_with("POST /usercenter/v3/get_my_info?"));
    assert!(!requests.iter().any(|value| value.contains(&input.password)));
    assert!(!requests[0].contains(&input.principal));
    let (headers, body) = requests[0].split_once("\r\n\r\n").unwrap();
    let target = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    let mut query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    let signature = query.remove("signature").unwrap();
    let query = query
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    assert_eq!(signature, android_signature(&query, body.as_bytes()));
}

#[tokio::test]
async fn native_password_profile_mismatch_does_not_issue_a_credential() {
    let cipher = ExchangeCipher::for_test(SEED);
    let (mut client, task) = server(vec![success(&cipher), json!({"status":1,"error_code":0,"data":{"userid":"987654321","nickname":"Different account"}})]).await;
    client.password_test_seed = Some(SEED.into());
    assert!(client.login_native_password(&request()).await.is_err());
    assert_eq!(task.await.unwrap().len(), 2);
}

#[tokio::test]
async fn native_password_invalid_inputs_are_rejected_before_network() {
    let client = KugouClient::new(&KugouConfig::default()).unwrap();
    let mut input = request();
    input.account = "named-server-account".to_owned();
    assert_eq!(
        client.login_native_password(&input).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    input = request();
    input.backend = tuneweave_core::PasswordLoginBackend::Web;
    assert_eq!(
        client.login_native_password(&input).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    input.backend = tuneweave_core::PasswordLoginBackend::Native;
    assert_eq!(
        client.login_web_password(&input).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    input = request();
    input.password_format = PasswordFormat::Md5;
    assert_eq!(
        client.login_native_password(&input).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    input = request();
    input.secure_captcha = Some("unbound-answer".to_owned());
    assert_eq!(
        client.login_native_password(&input).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}
