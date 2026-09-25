use super::*;
use crate::credential::MiguCredential;
use crate::provider::account_media::tests::{reply, setup};
use crate::provider::session::tests::{profile, read, stored};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 0xff, 0xd9];
const UPLOAD: usize = 7;

fn encrypted(value: serde_json::Value) -> String {
    let body = crate::client::native_http::encode(value.to_string().as_bytes());
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn native_profile(uid: &str) -> String {
    encrypted(
        json!({"code":"000000","userInfoItem":{"userId":uid,"middleIcon":"https://d.musicapp.migu.cn/old-avatar.jpg","msisdn":"do-not-retain-phone"}}),
    )
}
fn mode(value: u8) -> String {
    reply(
        json!({"code":"000000","data":{"avatarIconType":value}}),
        None,
    )
}
fn audit(types: serde_json::Value) -> String {
    reply(
        json!({"code":"000000","data":{"status":true,"types":types}}),
        None,
    )
}
fn frames() -> Vec<String> {
    vec![
        profile("111", "pacmtoken: profile-pacm\r\n"),
        reply(
            json!({"code":"000000","data":"native-token-fixture"}),
            Some("exchange-pacm"),
        ),
        profile("111", "pacmtoken: verified-pacm\r\n"),
        encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}})),
        native_profile("111"),
        mode(0),
        audit(json!(["9"])),
        reply(json!({"code":"000000"}), None),
        native_profile("111"),
        mode(0),
        audit(json!(["9", "4"])),
        profile("111", "pacmtoken: final-pacm\r\n"),
    ]
}
fn request(account: Option<&str>) -> ImageUploadRequest {
    ImageUploadRequest {
        filename: "avatar.jpg".into(),
        content_type: "image/jpeg".into(),
        data: JPEG.to_vec(),
        image_size: None,
        crop_x: None,
        crop_y: None,
        account: account.map(str::to_owned),
    }
}

struct Harness {
    provider: MiguProvider,
    requests: tokio::task::JoinHandle<Vec<Vec<u8>>>,
    seen: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
}
async fn server(responses: Vec<String>, gate: Option<usize>) -> Harness {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (seen_tx, seen) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let requests = tokio::spawn(async move {
        let mut requests = Vec::new();
        let mut gate_channels = Some((seen_tx, release_rx));
        for (index, response) in responses.into_iter().enumerate() {
            let bytes = tokio::time::timeout(Duration::from_secs(10), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                if gate == Some(index) {
                    let (seen_tx, release_rx) = gate_channels.take().unwrap();
                    seen_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                }
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
                bytes
            })
            .await
            .expect("avatar mock did not receive its expected request");
            requests.push(bytes);
        }
        requests
    });
    Harness {
        provider: MiguProvider::from_client(
            MiguClient::test_client().with_catalog_test_origin(origin),
        ),
        requests,
        seen,
        release,
    }
}
fn header(request: &[u8]) -> &str {
    let end = request.windows(4).position(|v| v == b"\r\n\r\n").unwrap();
    std::str::from_utf8(&request[..end]).unwrap()
}

#[tokio::test]
async fn account_avatar_upload_is_isolated_signed_and_reports_pending_review() {
    for selection in ["default", "named", "caller"] {
        let mut h = server(frames(), None).await;
        let (store, original, alias) = setup(&mut h.provider, selection);
        let result = h
            .provider
            .upload_account_avatar(&request(Some(alias)))
            .await
            .unwrap();
        assert_eq!(result.url, None);
        assert_eq!(result.image_id, None);
        assert_eq!(result.extensions["write_outcome"], "pending_review");
        assert_eq!(result.extensions["source_user_id"], "111");
        let output = serde_json::to_string(&result).unwrap();
        for secret in [
            "old-avatar",
            "native-token-fixture",
            "do-not-retain",
            "profile-pacm",
            "verified-pacm",
            "exchange-pacm",
            "final-pacm",
        ] {
            assert!(!output.contains(secret));
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if selection == "caller" {
            assert_eq!(read(&store, alias), original);
            let update = h.provider.take_response_credential().unwrap().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap().token(),
                "final-pacm"
            );
            assert!(!update.secret().contains("native-token-fixture"));
        } else {
            assert_eq!(read(&store, alias).token(), "final-pacm");
            assert_eq!(
                read(
                    &store,
                    if alias == "default" {
                        "personal"
                    } else {
                        "default"
                    }
                ),
                original
            );
        }
        let requests = h.requests.await.unwrap();
        assert_eq!(requests.len(), 12);
        assert_eq!(
            requests.iter().filter(|v| v.starts_with(b"POST ")).count(),
            1
        );
        let upload = &requests[UPLOAD];
        let headers = header(upload);
        assert!(
            headers.starts_with(
                "POST /MIGUM2.0/v1.0/picUpload.do?syncOtherApp=false&type=00 HTTP/1.1"
            )
        );
        assert!(headers.contains("content-type: image/jpeg\r\n"));
        assert_eq!(&upload[headers.len() + 4..], JPEG);
        for request in &requests[3..11] {
            let headers = header(request);
            for expected in [
                "token: native-token-fixture\r\n",
                "signversion: V005\r\n",
                "sign: ",
                "appid: music\r\n",
                "os: Android\r\n",
            ] {
                assert!(headers.contains(expected));
            }
            for forbidden in [
                "pacmtoken",
                "cookie:",
                "usessionid",
                "requestenc:",
                "responseenc:",
                "\r\nuid:",
            ] {
                assert!(!headers.to_ascii_lowercase().contains(forbidden));
            }
        }
        for at in [4, 8] {
            assert!(
                header(&requests[at])
                    .starts_with("GET /MIGUM3.0/user/user-info/v1.0?userId=111 HTTP/1.1")
            );
        }
        for at in [5, 9] {
            let target = header(&requests[at]).split_whitespace().nth(1).unwrap();
            let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
            let pairs = url
                .query_pairs()
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(pairs.len(), 3);
            assert_eq!(pairs["uid"], "111");
            assert_eq!(pairs["sceneType"], "crbt");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&pairs["cv"]).unwrap(),
                json!({"cvs":[{"cv":3,"styleId":"1"},{"cv":3,"styleId":"2"},{"cv":3,"styleId":"3"},{"cv":3,"styleId":"4"}]})
            );
        }
        assert!(
            requests
                .iter()
                .all(|r| !header(r).contains("convert/v1.0") && !header(r).contains("resourceId="))
        );
    }
}

#[tokio::test]
async fn account_avatar_preflight_rejects_wrong_uid_invalid_mode_or_pending_review() {
    for (at, replacement, code) in [
        (
            3,
            encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"222"}}})),
            ErrorCode::PermissionDenied,
        ),
        (4, native_profile("222"), ErrorCode::PermissionDenied),
        (
            4,
            encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}})),
            ErrorCode::UpstreamError,
        ),
        (5, mode(2), ErrorCode::UpstreamError),
        (
            5,
            reply(json!({"code":"000000","data":{}}), None),
            ErrorCode::UpstreamError,
        ),
        (6, audit(json!(["4"])), ErrorCode::Conflict),
        (6, audit(json!(null)), ErrorCode::UpstreamError),
        (6, audit(json!([4])), ErrorCode::UpstreamError),
    ] {
        let mut responses = frames();
        responses[at] = replacement;
        responses.truncate(at + 1);
        let mut h = server(responses, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        let failure = h
            .provider
            .upload_account_avatar(&request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(failure.code, code, "at {at}");
        assert!(failure.details.get("upload_requests_dispatched").is_none());
        assert!(
            h.requests
                .await
                .unwrap()
                .iter()
                .all(|r| !r.starts_with(b"POST "))
        );
    }
}

mod conversion;

#[tokio::test]
async fn account_avatar_ack_or_readback_failure_never_retries_or_claims_updated() {
    for (at, replacement, code) in [
        (
            7,
            reply(json!({"code":"200004","info":"native-token-fixture"}), None),
            ErrorCode::PermissionDenied,
        ),
        (
            7,
            reply(json!({"code":0,"info":"do-not-retain-phone"}), None),
            ErrorCode::UpstreamError,
        ),
        (
            7,
            reply(json!({"code":"native-token-fixture"}), None),
            ErrorCode::UpstreamError,
        ),
        (
            7,
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .into(),
            ErrorCode::RateLimited,
        ),
        (8, native_profile("222"), ErrorCode::PermissionDenied),
        (9, mode(1), ErrorCode::Conflict),
        (10, audit(json!(["9"])), ErrorCode::UpstreamError),
        (
            10,
            audit(json!(["native-token-fixture"])),
            ErrorCode::UpstreamError,
        ),
    ] {
        let mut responses = frames();
        responses[at] = replacement;
        responses.truncate(at + 1);
        let mut h = server(responses, None).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let mut failure = h
            .provider
            .upload_account_avatar(&request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(failure.code, code, "at {at}");
        assert!(!failure.retryable);
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert_eq!(failure.details["upload_requests_dispatched"], 1);
        assert_eq!(failure.details["automatic_retry"], false);
        assert!(!failure.message.contains("native-token-fixture"));
        assert!(!failure.details.to_string().contains("native-token-fixture"));
        let response_update = h.provider.take_response_credential().unwrap();
        let update = failure.take_caller_credential_update();
        if code == ErrorCode::Conflict {
            assert!(response_update.is_none());
            assert!(update.is_none());
        } else {
            // An upload failure does not revoke the previously verified PACM
            // rotation. Both delivery paths retain the same UID and generation.
            let expected = original.rotate("verified-pacm".into()).unwrap();
            for credential in [response_update.unwrap(), update.unwrap()] {
                assert_eq!(MiguCredential::parse_caller(&credential).unwrap(), expected);
                for secret in [
                    "native-token-fixture",
                    "do-not-retain-session",
                    "do-not-retain-phone",
                ] {
                    assert!(!credential.secret().contains(secret));
                }
            }
        }
        assert!(h.provider.take_response_credential().unwrap().is_none());
        assert!(failure.take_caller_credential_update().is_none());
        assert_eq!(read(&store, alias), original);
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        assert_eq!(
            h.requests
                .await
                .unwrap()
                .iter()
                .filter(|r| r.starts_with(b"POST "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn account_avatar_clear_invalid_image_and_missing_account_fail_before_network() {
    let mut h = server(vec![], None).await;
    let (_, _, alias) = setup(&mut h.provider, "named");
    let base = request(Some(alias));
    for bad in [
        ImageUploadRequest {
            data: vec![],
            ..base.clone()
        },
        ImageUploadRequest {
            content_type: "image/png".into(),
            ..base.clone()
        },
        ImageUploadRequest {
            data: b"invalid".to_vec(),
            ..base.clone()
        },
        ImageUploadRequest {
            crop_x: Some(0),
            ..base.clone()
        },
        ImageUploadRequest {
            image_size: Some(10),
            ..base.clone()
        },
        ImageUploadRequest {
            filename: "bad\n.jpg".into(),
            ..base.clone()
        },
    ] {
        assert_eq!(
            h.provider
                .upload_account_avatar(&bad)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        h.provider
            .upload_account_avatar(&request(Some("missing")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(h.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn account_avatar_final_account_change_discards_pending_result() {
    let mut responses = frames();
    responses[11] = profile("222", "");
    let mut h = server(responses, None).await;
    let (_, _, alias) = setup(&mut h.provider, "caller");
    let failure = h
        .provider
        .upload_account_avatar(&request(Some(alias)))
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::AuthenticationRequired);
    assert_eq!(failure.details["write_outcome"], "unconfirmed");
    assert!(h.provider.take_response_credential().unwrap().is_none());
    assert_eq!(h.requests.await.unwrap().len(), 12);
}

#[tokio::test]
async fn account_avatar_new_login_at_dispatch_preserves_replacement() {
    for at in [6, UPLOAD] {
        let mut responses = frames();
        responses.truncate(at + 1);
        let mut h = server(responses, Some(at)).await;
        let (store, _, alias) = setup(&mut h.provider, "named");
        let provider = h.provider.clone();
        let task =
            tokio::spawn(
                async move { provider.upload_account_avatar(&request(Some(alias))).await },
            );
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        let next = MiguCredential::verified("111".into(), "new-login-pacm".into()).unwrap();
        store.put(&stored(alias, &next)).unwrap();
        h.release.send(()).unwrap();
        let failure = task.await.unwrap().unwrap_err();
        assert_eq!(failure.code, ErrorCode::Conflict);
        if at == UPLOAD {
            assert_eq!(failure.details["write_outcome"], "unconfirmed");
        } else {
            assert!(failure.details.get("upload_requests_dispatched").is_none());
        }
        assert_eq!(read(&store, alias), next);
        let requests = h.requests.await.unwrap();
        assert_eq!(requests.len(), at + 1);
        assert_eq!(
            requests.iter().filter(|r| r.starts_with(b"POST ")).count(),
            usize::from(at == UPLOAD)
        );
    }
}

#[tokio::test]
async fn account_avatar_cancel_clears_caller_updates_before_and_after_dispatch() {
    for at in [6, UPLOAD] {
        let mut responses = frames();
        responses.truncate(at + 1);
        let mut h = server(responses, Some(at)).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let provider = h.provider.clone();
        let task =
            tokio::spawn(
                async move { provider.upload_account_avatar(&request(Some(alias))).await },
            );
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(h.provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, alias), original);
        h.requests.abort();
    }
}
