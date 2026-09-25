use super::*;
use crate::provider::account_media::tests::setup as setup_mode;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::ImageUploadRequest;

const OLD_COVER: &str = "https://d.musicapp.migu.cn/data/oss/old-cover.png";
const NEW_COVER: &str = "https://d.musicapp.migu.cn/data/oss/new-cover.png";
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 0xff, 0xd9];

fn flow(after_cover: &str, upload_code: &str) -> Flow {
    let mut f = Flow::new();
    f.profile();
    f.data("before_home", home("999"));
    let created: Vec<_> = std::iter::once(77).chain(1..=20).collect();
    f.library("before_library", &created, false);
    let tracks: Vec<_> = (1..=50).chain([1]).collect();
    f.snapshot(false, &tracks);
    for (label, value) in f.labels.iter().zip(&mut f.values) {
        if matches!(*label, "before_metadata" | "before_metadata_final") {
            value["data"]["originalImgUrl"] = json!(OLD_COVER);
        }
    }
    f.push(
        "native_profile",
        json!({"code":"000000","data":{"userId":"111","nickName":"Account","usessionId":"native-session-fixture"}}),
    );
    f.pair(
        "exchange",
        json!({"code":"000000","data":"native-token-fixture"}),
    );
    f.push(
        "validate",
        json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}}),
    );
    f.push("upload", json!({"code":upload_code}));
    f.snapshot(true, &tracks);
    for (label, value) in f.labels.iter().zip(&mut f.values) {
        if matches!(*label, "after_metadata" | "after_metadata_final") {
            value["data"]["originalImgUrl"] = json!(after_cover);
        }
    }
    f.library("after_library", &created, false);
    f.data("after_home", home("999"));
    f
}

fn wire(f: &Flow) -> Vec<Vec<u8>> {
    let mut replies = f
        .wire()
        .into_iter()
        .map(String::into_bytes)
        .collect::<Vec<_>>();
    if let Some(at) = f.labels.iter().position(|label| *label == "validate") {
        let body = crate::client::native_http::encode(f.values[at].to_string().as_bytes());
        replies[at] = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes();
    }
    replies
}

async fn binary_server(
    responses: Vec<Vec<u8>>,
) -> (MiguProvider, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let request = tokio::time::timeout(Duration::from_secs(10), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                socket.write_all(&response).await.unwrap();
                socket.shutdown().await.unwrap();
                bytes
            })
            .await
            .unwrap();
            requests.push(request);
        }
        requests
    });
    (
        MiguProvider::from_client(MiguClient::test_client().with_catalog_test_origin(origin)),
        task,
    )
}

fn request(alias: Option<&str>) -> ImageUploadRequest {
    ImageUploadRequest {
        filename: "playlist-cover.jpg".into(),
        content_type: "image/jpeg".into(),
        data: JPEG.to_vec(),
        image_size: None,
        crop_x: None,
        crop_y: None,
        account: alias.map(str::to_owned),
    }
}

#[tokio::test]
async fn native_cover_upload_sends_raw_jpeg_and_requires_complete_readback() {
    for mode in ["default", "named", "caller"] {
        let f = flow(NEW_COVER, "000000");
        let count = f.values.len();
        let upload = f
            .labels
            .iter()
            .position(|label| *label == "upload")
            .unwrap();
        let (mut provider, requests) = binary_server(wire(&f)).await;
        let (store, original, alias) = setup_mode(&mut provider, mode);
        let result = provider
            .update_playlist_cover("77", &request(Some(alias)))
            .await
            .unwrap();
        assert_eq!(result.image.url.as_deref(), Some(NEW_COVER));
        assert_eq!(result.image.image_id, None);
        assert_eq!(
            result.extensions["verified_by"],
            "complete_playlist_and_created_library_readback"
        );
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), count);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with(b"POST "))
                .count(),
            1,
            "picUpload saves the cover directly; no second save request is expected"
        );
        let upload_request = &requests[upload];
        let split = upload_request
            .windows(4)
            .position(|value| value == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&upload_request[..split]).unwrap();
        assert!(
            headers.starts_with("POST /MIGUM2.0/v1.0/picUpload.do?resourceId=77&type=02 HTTP/1.1")
        );
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("content-type: image/jpeg\r\n")
        );
        assert!(headers.contains("token: native-token-fixture\r\n"));
        assert!(headers.contains("signversion: V005\r\n"));
        assert!(headers.contains("sign: "));
        assert_eq!(&upload_request[split + 4..], JPEG);
        let lowercase_headers = headers.to_ascii_lowercase();
        for forbidden in [
            "pacmtoken:",
            "native-session-fixture",
            "cookie:",
            "\r\nuid:",
        ] {
            let forbidden = forbidden.to_ascii_lowercase();
            assert!(!lowercase_headers.contains(forbidden.as_str()));
        }
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("native-token-fixture"));
        assert!(!encoded.contains("native-session-fixture"));
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            let credential = provider.take_response_credential().unwrap().unwrap();
            assert!(!credential.secret().contains("native-token-fixture"));
            assert!(!credential.secret().contains("native-session-fixture"));
        }
    }
}

#[tokio::test]
async fn native_cover_upload_rejects_non_jpeg_before_account_selection() {
    let provider = MiguProvider::from_client(MiguClient::test_client());
    let mut invalid = request(None);
    invalid.data = b"not a jpeg".to_vec();
    let error = provider
        .update_playlist_cover("77", &invalid)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
}

#[tokio::test]
async fn native_cover_upload_does_not_accept_ack_without_a_changed_cover() {
    let mut f = flow(OLD_COVER, "000000");
    f.truncate(f.at("after_library"));
    let upload = f
        .labels
        .iter()
        .position(|label| *label == "upload")
        .unwrap();
    let (mut provider, requests) = binary_server(wire(&f)).await;
    let (store, _, alias) = setup_mode(&mut provider, "named");
    let error = provider
        .update_playlist_cover("77", &request(Some(alias)))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(error.details["upload_requests_dispatched"], 1);
    assert!(!error.retryable);
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), f.values.len());
    let split = requests[upload]
        .windows(4)
        .position(|value| value == b"\r\n\r\n")
        .unwrap();
    assert_eq!(&requests[upload][split + 4..], JPEG);
    assert_eq!(
        read(&store, alias).token(),
        format!("p{}", f.values.len() - 1)
    );
}

#[tokio::test]
async fn native_cover_upload_rejects_unstable_complete_metadata_reads() {
    for after in [false, true] {
        for field in ["cover", "tag_identity", "custom_cover_marker"] {
            let mut f = flow(NEW_COVER, "000000");
            for (label, value) in f.labels.iter().zip(&mut f.values) {
                if matches!(
                    *label,
                    "before_metadata"
                        | "before_metadata_final"
                        | "after_metadata"
                        | "after_metadata_final"
                ) {
                    value["data"]["tags"] = json!([{"tagId":"100","tagName":"流行"}]);
                    value["data"]["havePrivatePic"] = json!("01");
                }
            }
            let final_read = f.at(if after {
                "after_metadata_final"
            } else {
                "before_metadata_final"
            });
            let value = &mut f.values[final_read]["data"];
            match field {
                "cover" => {
                    value["originalImgUrl"] = json!(if after { OLD_COVER } else { NEW_COVER });
                }
                "tag_identity" => value["tags"][0]["tagId"] = json!("200"),
                _ => value["havePrivatePic"] = json!("00"),
            }
            f.truncate(f.at(if after {
                "after_library"
            } else {
                "native_profile"
            }));
            let (mut provider, requests) = binary_server(wire(&f)).await;
            let (store, _, alias) = setup_mode(&mut provider, "named");
            let failure = provider
                .update_playlist_cover("77", &request(Some(alias)))
                .await
                .unwrap_err();
            assert_eq!(failure.code, ErrorCode::UpstreamError, "{after}/{field}");
            if after {
                assert_eq!(failure.details["write_outcome"], "unconfirmed");
                assert_eq!(failure.details["upload_requests_dispatched"], 1);
                assert!(!failure.retryable);
            } else {
                assert!(failure.details.get("write_outcome").is_none());
                assert!(failure.message.contains("pre-upload read"));
            }
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), f.values.len());
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.starts_with(b"POST "))
                    .count(),
                usize::from(after)
            );
            assert_eq!(
                read(&store, alias).token(),
                format!("p{}", f.values.len() - 1)
            );
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        }
    }
}

#[tokio::test]
async fn native_cover_upload_requires_strict_ack_and_never_retries_rejected_uploads() {
    for code in [
        json!("200004"),
        json!("999999"),
        json!(0),
        serde_json::Value::Null,
    ] {
        let mut f = flow(NEW_COVER, "000000");
        let upload = f.at("upload");
        f.values[upload] =
            json!({"code":code,"info":"native-token-fixture native-session-fixture"});
        f.truncate(upload + 1);
        let (mut provider, requests) = binary_server(wire(&f)).await;
        let (store, original, alias) = setup_mode(&mut provider, "caller");
        let failure = provider
            .update_playlist_cover("77", &request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(
            failure.code,
            if code == json!("200004") {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert_eq!(failure.details["upload_requests_dispatched"], 1);
        assert_eq!(failure.details["automatic_retry"], false);
        assert!(!failure.retryable);
        let error_text = format!("{} {}", failure.message, failure.details);
        assert!(!error_text.contains("native-token-fixture"));
        assert!(!error_text.contains("native-session-fixture"));
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), f.values.len());
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with(b"POST "))
                .count(),
            1
        );
        assert_eq!(read(&store, alias), original);
        let credential = provider.take_response_credential().unwrap().unwrap();
        assert!(!credential.secret().contains("native-token-fixture"));
        assert!(!credential.secret().contains("native-session-fixture"));
    }
}

#[tokio::test]
async fn native_cover_upload_cancellation_discards_pending_caller_update() {
    for after_upload in [false, true] {
        let mut f = flow(NEW_COVER, "000000");
        if !after_upload {
            f.truncate(f.at("validate") + 1);
        }
        // Only responses are UTF-8. The shared gate retains request bodies as
        // bytes, including the JPEG sent before the final identity read.
        let responses = wire(&f)
            .into_iter()
            .map(|response| String::from_utf8(response).unwrap())
            .collect();
        let (mut provider, seen, release, server) = gated(responses).await;
        let (store, original, alias) = setup_mode(&mut provider, "caller");
        let provider = Arc::new(provider);
        let running = provider.clone();
        let request = request(Some(alias));
        let task = tokio::spawn(async move { running.update_playlist_cover("77", &request).await });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .expect("cover flow did not reach the requested cancellation gate")
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, alias), original);
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        drop(release);
        server.abort();
        let _ = server.await;
    }
}
