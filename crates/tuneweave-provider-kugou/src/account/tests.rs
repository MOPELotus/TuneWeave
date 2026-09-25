use super::*;
use crate::{KugouConfig, device::KugouDevice};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const UID: &str = "123456789";
const TOKEN: &str = "synthetic-original-token";
const ROTATED: &str = "synthetic-rotated-token";

pub(super) fn session(client: KugouLoginClient) -> NativeSession {
    NativeSession {
        client,
        device: KugouDevice::default().identity(),
        user_id: UID.to_owned(),
        token: TOKEN.to_owned(),
        vip_token: None,
        t1: None,
    }
}
fn authorization(session: &NativeSession) -> KugouQrAuthorization {
    KugouQrAuthorization::test_authorization(
        session.client,
        session.device.clone(),
        session.user_id.clone(),
        session.token.clone(),
    )
}
fn envelope(data: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"status":1,"error_code":0,"data":data})).unwrap()
}
pub(super) fn ok(data: Value) -> String {
    frame(200, "Content-Type: application/json\r\n", envelope(data))
}
pub(super) fn frame(status: u16, headers: &str, body: Vec<u8>) -> String {
    format!(
        "HTTP/1.1 {status} Test\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        String::from_utf8(body).unwrap()
    )
}
fn exchanged() -> Value {
    json!({"userid":UID,"token":ROTATED,"vip_token":"synthetic-vip-token","t1":"synthetic-device-token"})
}
fn profile() -> Value {
    json!({"nickname":"测试用户","pic":"https://imge.kugou.com/kugouicon/{size}/test.jpg","servertime":1700000000})
}

pub(super) async fn server(
    frames: Vec<String>,
) -> (KugouClient, tokio::task::JoinHandle<Vec<String>>) {
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
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                assert!(request.len() < 262144);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&request[..end]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            // A rejected Content-Length can close the connection before its body is sent.
            let response = super::response_bytes_for_request(&request, &frame);
            requests.push(String::from_utf8(request).unwrap());
            let _ = socket.write_all(&response).await;
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
    (client, task)
}

pub(super) fn request(
    request: &str,
    endpoint: Endpoint,
    client: KugouLoginClient,
) -> (BTreeMap<String, String>, Value) {
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    assert!(head.starts_with("POST "));
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    assert_eq!(url.path(), endpoint.path());
    let mut p: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    let signature = p.remove("signature").unwrap();
    let signed: BTreeMap<&str, String> = p.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let expected = match client {
        KugouLoginClient::Standard => android_signature(&signed, body.as_bytes()),
        KugouLoginClient::Concept => concept_signature(&signed, body.as_bytes()),
        KugouLoginClient::Web => panic!("Web may not use native exchange"),
    };
    assert_eq!(signature, expected);
    assert_eq!(p["appid"], client.appid().to_string());
    assert_eq!(p["clientver"], client.clientver().to_string());
    for header in [
        "cookie:",
        "authorization:",
        "x-real-ip:",
        "x-forwarded-for:",
    ] {
        assert!(!head.to_ascii_lowercase().contains(header));
    }
    if matches!(endpoint, Endpoint::LibraryTracks | Endpoint::LibraryRemove) {
        assert!(
            head.to_ascii_lowercase()
                .contains("x-router: cloudlist.service.kugou.com\r\n")
        );
    } else {
        assert!(!head.to_ascii_lowercase().contains("x-router:"));
    }
    let body = if matches!(endpoint, Endpoint::ListDeleteStandard) {
        Value::String(body.to_owned())
    } else {
        serde_json::from_str(body).unwrap()
    };
    (p, body)
}

#[tokio::test]
async fn native_qr_completion_verifies_exchange_then_profile_with_the_rotated_token() {
    for kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        let original = session(kind);
        let dfid = "0123456789ABCDEF01234567";
        let mut token_data = exchanged();
        token_data["dfid"] = json!(dfid);
        let (client, task) = server(vec![ok(token_data), ok(profile())]).await;
        let result = client
            .complete_qr_login(authorization(&original))
            .await
            .unwrap();
        assert_eq!(result.profile.user_id.as_deref(), Some(UID));
        assert_eq!(result.profile.account, "default");
        assert_eq!(result.profile.nickname.as_deref(), Some("测试用户"));
        assert_eq!(
            result.profile.avatar_url.as_deref(),
            Some("https://imge.kugou.com/kugouicon/400/test.jpg")
        );
        assert!(result.profile.authenticated);
        assert!(result.profile.extensions.is_empty());
        let credential =
            KugouCredential::parse_caller(result.credential.as_ref().unwrap()).unwrap();
        assert_eq!(credential.session.token, ROTATED);
        assert_eq!(
            credential.session.vip_token.as_deref(),
            Some("synthetic-vip-token")
        );
        assert_eq!(
            credential.session.t1.as_deref(),
            Some("synthetic-device-token")
        );
        assert_eq!(credential.session.device.guid, original.device.guid);
        assert_eq!(credential.session.device.mid, original.device.mid);
        assert_eq!(credential.session.device.dfid.as_deref(), Some(dfid));
        let requests = task.await.unwrap();
        assert_eq!(requests.len(), 2);
        let (p, body) = request(&requests[0], Endpoint::Exchange, kind);
        assert_eq!(p["token"], TOKEN);
        assert_eq!(p["userid"], UID);
        assert_eq!(p["dfid"], "-");
        assert_eq!(p["mid"], original.device.mid);
        assert_eq!(
            body["clienttime_ms"].as_u64().unwrap() / 1000,
            p["clienttime"].parse::<u64>().unwrap()
        );
        assert_eq!(
            body["p3"],
            crypto::p3(&original, p["clienttime"].parse().unwrap()).unwrap()
        );
        assert!(
            !requests[0]
                .split_once("\r\n\r\n")
                .unwrap()
                .1
                .contains(TOKEN)
        );
        if kind == KugouLoginClient::Standard {
            assert_eq!(body["t1"], 0);
            assert_eq!(body["t2"], 0);
            assert!(body.get("dev").is_none());
        } else {
            let (t1, t2) =
                crypto::concept_fingerprints(&original, body["clienttime_ms"].as_u64().unwrap())
                    .unwrap();
            assert_eq!(body["t1"], t1);
            assert_eq!(body["t2"], t2);
            assert_eq!(body["dev"], "TuneWeave");
        }
        let (p, body) = request(&requests[1], Endpoint::Profile, kind);
        assert_eq!(p["token"], ROTATED);
        assert_eq!(p["dfid"], dfid);
        assert_eq!(p["mid"], original.device.mid);
        assert_eq!(p["plat"], "1");
        assert_eq!(
            body["p"],
            crypto::profile_p(kind, ROTATED, p["clienttime"].parse().unwrap()).unwrap()
        );
        assert_eq!(body["userid"], UID.parse::<u64>().unwrap());
        let debug = format!("{result:?} {credential:?} {:?}", credential.session);
        for secret in [
            TOKEN,
            ROTATED,
            "synthetic-vip-token",
            "synthetic-device-token",
            original.device.guid.as_str(),
        ] {
            assert!(!debug.contains(secret));
        }
    }
}

#[tokio::test]
async fn login_does_not_export_an_account_when_either_identity_check_fails() {
    for frames in [
        vec![ok(json!({"userid":"999","token":ROTATED}))],
        vec![ok(json!({"token":ROTATED}))],
        vec![
            ok(exchanged()),
            ok(json!({"userid":"999","nickname":"wrong user"})),
        ],
        vec![ok(exchanged()), ok(json!({}))],
        vec![
            ok(exchanged()),
            frame(
                500,
                "Content-Type: text/plain\r\n",
                b"synthetic-private-error".to_vec(),
            ),
        ],
    ] {
        let count = frames.len();
        let (client, task) = server(frames).await;
        let mut failure = client
            .complete_qr_login(authorization(&session(KugouLoginClient::Standard)))
            .await
            .unwrap_err();
        assert!(failure.take_caller_credential_update().is_none());
        assert!(!format!("{failure:?}").contains("synthetic-private-error"));
        assert_eq!(task.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn explicit_refresh_preserves_generation_and_returns_verified_rotation_on_later_failure() {
    let source = KugouCredential::verified(session(KugouLoginClient::Concept))
        .unwrap()
        .caller()
        .unwrap();
    for (response, expected_error, update_allowed) in [
        (ok(profile()), None, true),
        (
            frame(
                503,
                "Content-Type: text/plain\r\n",
                b"private-error".to_vec(),
            ),
            Some(ErrorCode::UpstreamError),
            true,
        ),
        (
            ok(json!({"userid":"999","nickname":"wrong"})),
            Some(ErrorCode::Conflict),
            false,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                br#"{"status":0,"error_code":20017,"data":null}"#.to_vec(),
            ),
            Some(ErrorCode::AuthenticationRequired),
            false,
        ),
    ] {
        let (client, task) = server(vec![ok(exchanged()), response]).await;
        let output = client.refresh_native_login(&source).await;
        let updated = if let Some(code) = expected_error {
            let mut failure = output.unwrap_err();
            assert_eq!(failure.code, code);
            failure.take_caller_credential_update()
        } else {
            output.unwrap().credential
        };
        assert_eq!(updated.is_some(), update_allowed);
        if let Some(updated) = updated {
            let before: Value = serde_json::from_str(source.secret()).unwrap();
            let after: Value = serde_json::from_str(updated.secret()).unwrap();
            assert_eq!(before["generation"], after["generation"]);
            assert_eq!(after["session"]["token"], ROTATED);
            assert_ne!(source, updated);
        }
        assert_eq!(task.await.unwrap().len(), 2);
    }
}

#[test]
fn encrypted_exchange_requires_returned_uid_and_rejects_conflicting_or_duplicate_fields() {
    let cipher = ExchangeCipher::random().unwrap();
    let original = session(KugouLoginClient::Standard);
    for inner in [
        br#"{"userid":123456789,"token":"synthetic-rotated-token"}"#.as_slice(),
        br#""synthetic-rotated-token""#.as_slice(),
        b"synthetic-rotated-token".as_slice(),
    ] {
        let bytes = envelope(json!({"userid":UID,"secu_params":cipher.encrypt(inner).unwrap()}));
        assert_eq!(
            parse_exchange(&bytes, &original, &cipher).unwrap().token,
            ROTATED
        );
    }
    for inner in [
        br#"{"userid":"999","token":"synthetic-rotated-token"}"#.as_slice(),
        br#"{"userid":"123456789","token":"a","token":"b"}"#.as_slice(),
        br#"{"userid":"123456789","token":null}"#.as_slice(),
        br#"{"userid":"123456789","token":"synthetic-rotated-token","secu_params":"recursive"}"#
            .as_slice(),
        b"[malformed-token]".as_slice(),
        b" synthetic-rotated-token ".as_slice(),
        br#"{"token":"truncated""#.as_slice(),
    ] {
        let bytes = envelope(json!({"userid":UID,"secu_params":cipher.encrypt(inner).unwrap()}));
        assert!(parse_exchange(&bytes, &original, &cipher).is_err());
    }
    let encrypted = cipher
        .encrypt(br#"{"userid":"123456789","token":"synthetic-rotated-token"}"#)
        .unwrap();
    for outer in [
        json!({"userid":UID,"token":"different-token","secu_params":encrypted}),
        json!({"userid":"999","secu_params":encrypted}),
        json!({"uid":"999","secu_params":encrypted}),
        json!({"userid":null,"secu_params":encrypted}),
        json!({"token":ROTATED}),
        json!({"userid":UID}),
        json!({"userid":UID,"token":ROTATED,"dfid":"invalid"}),
    ] {
        assert!(parse_exchange(&envelope(outer), &original, &cipher).is_err());
    }
    for raw in [
        r#"{"status":1,"status":1,"error_code":0,"data":{"userid":"123456789","token":"a"}}"#,
        r#"{"status":1,"error_code":0,"data":{"userid":"123456789","token":"a","token":"b"}}"#,
    ] {
        assert!(parse_exchange(raw.as_bytes(), &original, &cipher).is_err());
    }
    let bare = envelope(json!({"secu_params":cipher.encrypt(b"synthetic-token").unwrap()}));
    assert!(parse_exchange(&bare, &original, &cipher).is_err());
}

#[test]
fn exchange_keeps_vip_and_device_tokens_distinct_and_honors_explicit_removal() {
    let cipher = ExchangeCipher::random().unwrap();
    let mut original = session(KugouLoginClient::Concept);
    original.vip_token = Some("old-vip".to_owned());
    original.t1 = Some("old-t1".to_owned());
    let minimal = envelope(json!({"userid":UID,"token":ROTATED}));
    let kept = parse_exchange(&minimal, &original, &cipher).unwrap();
    assert_eq!(kept.vip_token, original.vip_token);
    assert_eq!(kept.t1, original.t1);
    for fields in [
        json!({"userid":UID,"token":ROTATED,"vip_token":null,"t1":""}),
        json!({"userid":UID,"token":ROTATED,"vip_token":"","t1":null}),
    ] {
        let cleared = parse_exchange(&envelope(fields), &original, &cipher).unwrap();
        assert!(cleared.vip_token.is_none());
        assert!(cleared.t1.is_none());
        assert_eq!(cleared.token, ROTATED);
    }
    let encrypted = cipher.encrypt(br#"{"vip_token":"other-vip"}"#).unwrap();
    assert!(parse_exchange(&envelope(json!({"userid":UID,"token":ROTATED,"vip_token":"outer-vip","secu_params":encrypted})),&original,&cipher).is_err());
}

#[test]
fn self_profile_rejects_identity_and_known_field_errors_without_inventing_membership() {
    let verified = session(KugouLoginClient::Standard);
    for value in [
        json!({}),
        json!({"userid":"999"}),
        json!({"uid":"999","nickname":"wrong user"}),
        json!({"user_id":"999","nickname":"wrong user"}),
        json!({"userid":UID,"uid":"999","nickname":"wrong user"}),
        json!({"userid":"0123456789"}),
        json!({"userid":null,"nickname":"name"}),
        json!({"nickname":123}),
        json!({"nickname":"a\nb"}),
        json!({"nickname":"x".repeat(513)}),
        json!({"pic":"https://untrusted.invalid/avatar"}),
        json!({"vip_type":1}),
    ] {
        assert!(parse_profile(&envelope(value), &verified).is_err());
    }
    let p=parse_profile(&envelope(json!({"userid":123456789,"nickname":"name","vip_type":1,"vip_token":"private","username":"private-contact"})),&verified).unwrap();
    assert_eq!(
        p.extensions,
        BTreeMap::from([("vip_type".into(), json!(1))])
    );
    assert!(!format!("{p:?}").contains("private"));
    assert!(
        parse_profile(
            br#"{"status":1,"error_code":0,"data":{"userid":123456789,"userid":999}}"#,
            &verified
        )
        .is_err()
    );
}

#[test]
fn native_credentials_bind_client_uid_and_device_and_reject_untrusted_shapes() {
    let original = KugouCredential::verified(session(KugouLoginClient::Standard)).unwrap();
    let caller = original.caller().unwrap();
    assert_eq!(KugouCredential::parse_caller(&caller).unwrap(), original);
    let second = KugouCredential::verified(original.session.clone())
        .unwrap()
        .caller()
        .unwrap();
    assert_ne!(caller, second);
    let base: Value = serde_json::from_str(caller.secret()).unwrap();
    for (path, value) in [
        ("/version", json!(2)),
        ("/generation", json!("A".repeat(64))),
        ("/session/client", json!("web")),
        ("/session/user_id", json!("01")),
        ("/session/token", json!("bad\nsecret")),
        ("/session/vip_token", json!("undefined")),
        ("/session/device/mid", json!("99")),
        ("/session/device/dfid", json!("-")),
    ] {
        let mut changed = base.clone();
        *changed.pointer_mut(path).unwrap() = value;
        let supplied = ProviderCredential::new(
            Platform::Kugou,
            "kugou_native_v1",
            changed.to_string(),
            None,
        )
        .unwrap();
        assert!(KugouCredential::parse_caller(&supplied).is_err());
    }
    for path in ["", "/session", "/session/device"] {
        let mut changed = base.clone();
        changed
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("url".to_owned(), json!("https://untrusted.invalid"));
        let supplied = ProviderCredential::new(
            Platform::Kugou,
            "kugou_native_v1",
            changed.to_string(),
            None,
        )
        .unwrap();
        assert!(KugouCredential::parse_caller(&supplied).is_err());
    }
    let repeated = caller
        .secret()
        .replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    let supplied =
        ProviderCredential::new(Platform::Kugou, "kugou_native_v1", repeated, None).unwrap();
    assert!(KugouCredential::parse_caller(&supplied).is_err());
    for (platform, kind, expiry) in [
        (Platform::Migu, "kugou_native_v1", None),
        (Platform::Kugou, "cookie", None),
        (Platform::Kugou, "kugou_native_v1", Some(1700000000)),
    ] {
        let supplied = ProviderCredential::new(platform, kind, caller.secret(), expiry).unwrap();
        assert!(KugouCredential::parse_caller(&supplied).is_err());
    }
    let mut changed = original.session.clone();
    changed.user_id = "999".to_owned();
    assert!(original.rotate(changed).is_err());
    let mut changed = original.session.clone();
    changed.client = KugouLoginClient::Concept;
    assert!(original.rotate(changed).is_err());
    let mut changed = original.session.clone();
    changed.device = KugouDevice::default().identity();
    assert!(original.rotate(changed).is_err());
}

#[tokio::test]
async fn web_completion_and_malformed_caller_credentials_never_send_native_requests() {
    let (client, task) = server(vec![]).await;
    assert_eq!(
        client
            .exchange_qr_authorization(authorization(&session(KugouLoginClient::Web)))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    let bad = ProviderCredential::new(Platform::Kugou, "cookie", "synthetic-cookie", None).unwrap();
    assert_eq!(
        client.refresh_native_login(&bad).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(task.await.unwrap().is_empty());
}

#[tokio::test]
async fn native_transport_rejects_redirects_challenges_limits_and_non_json_without_retries() {
    let oversized = " ".repeat(RESPONSE_LIMIT + 1);
    let chunked = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{oversized}\r\n0\r\n\r\n",
        oversized.len()
    );
    for (response, code) in [
        (chunked, ErrorCode::UpstreamError),
        (
            frame(
                302,
                "Location: https://untrusted.invalid/credential-sink\r\n",
                vec![],
            ),
            ErrorCode::UpstreamError,
        ),
        (
            frame(429, "Retry-After: 9999\r\n", vec![]),
            ErrorCode::RateLimited,
        ),
        (frame(401, "", vec![]), ErrorCode::AuthenticationRequired),
        (
            frame(
                200,
                "SSA-CODE: synthetic-challenge\r\nContent-Type: application/json\r\n",
                envelope(exchanged()),
            ),
            ErrorCode::PermissionDenied,
        ),
        (
            frame(
                200,
                "Content-Type: text/html\r\n",
                b"synthetic-private-response".to_vec(),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                vec![b' '; RESPONSE_LIMIT + 1],
            ),
            ErrorCode::UpstreamError,
        ),
    ] {
        let (client, task) = server(vec![response]).await;
        let failure = client
            .complete_qr_login(authorization(&session(KugouLoginClient::Standard)))
            .await
            .unwrap_err();
        assert_eq!(failure.code, code);
        assert!(!format!("{failure:?}").contains("synthetic"));
        if code == ErrorCode::RateLimited {
            assert_eq!(failure.details["retry_after_secs"], 300);
        }
        assert_eq!(task.await.unwrap().len(), 1);
    }
}
