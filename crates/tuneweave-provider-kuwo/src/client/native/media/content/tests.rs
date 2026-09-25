use super::super::tests as data;
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

pub(crate) fn samples() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("audio-fixtures.json")).unwrap()["fixtures"]
        .as_array()
        .unwrap()
        .clone()
}
pub(crate) fn flow(sample: &Value, encrypted: bool) -> (Vec<Vec<u8>>, StreamRequest) {
    let spec = if sample["quality"] == "master" {
        MASTER
    } else if sample["quality"] == "hires" {
        HI_RES
    } else {
        match sample["format"].as_str().unwrap() {
            "mp3" => STANDARD_SPEC,
            "aac" => LOW,
            _ => LOSSLESS,
        }
    };
    let mut rights = data::rights();
    rights["songs"][0]["audio"][0]["quality"] = json!(spec.tag);
    rights["songs"][0]["audio"][0]["br"] = json!(spec.selector);
    rights["songs"][0]["audio"][0]["fmt"] = json!(spec.rights_format);
    let mut media = data::media();
    media["data"]["format"] = json!(spec.format);
    media["data"]["bitrate"] = json!(spec.selector);
    media["data"]["quality"] = json!(spec.tag);
    media["data"]["url"] = json!("");
    media["data"]["surl"] = json!(format!(
        "https://er-sycdn.kuwo.cn/token/time/file.{}",
        sample["transport_format"]
            .as_str()
            .unwrap_or_else(|| sample["extension"].as_str().unwrap())
    ));
    media["data"]["ekey"] = if encrypted {
        sample["ekey"].clone()
    } else {
        json!("")
    };
    let bytes = STANDARD
        .decode(
            sample[if encrypted {
                "cipher_base64"
            } else {
                "plain_base64"
            }]
            .as_str()
            .unwrap(),
        )
        .unwrap();
    (
        vec![
            json_response(&json!({"result":"ok"})),
            data::rights_reply(&rights),
            json_response(&media),
            response(
                200,
                "application/octet-stream",
                "Set-Cookie: sid=cdn-secret; Path=/\r\n",
                &bytes,
            ),
        ],
        StreamRequest {
            quality: spec.quality,
            ..data::request(None)
        },
    )
}
const STANDARD_SPEC: Spec = super::super::STANDARD;
fn input() -> KuwoNativeSessionInput {
    fixture::credential_fixture("42", "selected-session")
        .input()
        .unwrap()
}
fn control() -> Control {
    Control {
        cancelled: AtomicBool::new(false),
        deadline: std::time::Instant::now() + Duration::from_secs(30),
    }
}

#[test]
fn independently_encoded_real_audio_roundtrips_and_matches_declared_container() {
    for sample in samples() {
        let key = Key::parse(sample["ekey"].as_str().unwrap(), &input()).unwrap();
        let mut cipher = STANDARD
            .decode(sample["cipher_base64"].as_str().unwrap())
            .unwrap();
        for (i, chunk) in cipher.chunks_mut(137).enumerate() {
            key.transform((i * 137) as u64, chunk).unwrap();
        }
        let plain = STANDARD
            .decode(sample["plain_base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(cipher, plain);
        let declared = sample["format"].as_str().unwrap();
        let (_, ext) = format::inspect(&plain, declared, &control()).unwrap();
        assert_eq!(ext, sample["extension"]);
        assert!(
            format::inspect(
                &plain,
                if declared == "mp3" { "flac" } else { "mp3" },
                &control()
            )
            .is_err()
        );
        for size in [0, 1, 8, plain.len() - 1, plain.len() / 2] {
            assert!(
                format::inspect(&plain[..size], declared, &control()).is_err(),
                "format={declared}, size={size}"
            );
        }
        assert!(
            Key::parse(
                sample["ekey"].as_str().unwrap(),
                &KuwoNativeSessionInput::new(
                    "42",
                    "selected-session",
                    "1234567890",
                    "another-installation"
                )
                .unwrap()
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn native_content_play_and_download_deliver_exact_bytes_with_separate_authorization() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for sample in samples() {
        for encrypted in [false, true] {
            if sample["quality"] == "master" && !encrypted {
                continue;
            }
            for action in [Action::Play, Action::Download] {
                let (responses, request) = flow(&sample, encrypted);
                let mut f = fixture::setup(responses).await;
                let content = f
                    .client
                    .native_content(&credential, &data::track(), &request, action)
                    .await
                    .unwrap();
                assert_eq!(
                    content.bytes,
                    STANDARD
                        .decode(sample["plain_base64"].as_str().unwrap())
                        .unwrap()
                );
                assert_eq!(
                    content.filename,
                    format!("kuwo-67474.{}", sample["extension"].as_str().unwrap())
                );
                let calls = fixture::requests(&mut f, 4).await;
                assert_eq!(data::query(&calls[1])["action"], action.rights());
                assert_eq!(data::query(&calls[2])["mode"], action.mode());
                let cdn = calls[3].to_ascii_lowercase();
                for hidden in [
                    "selected-session",
                    "loginsid",
                    "cookie:",
                    "authorization:",
                    "referer:",
                    "cdn-secret",
                ] {
                    assert!(!cdn.contains(hidden));
                }
                assert!(!cdn.lines().next().unwrap().contains('?'));
                assert!(!format!("{content:?}").contains(sample["ekey"].as_str().unwrap()));
            }
        }
    }
}

#[tokio::test]
async fn encrypted_urls_and_keys_never_escape_url_operations() {
    let sample = &samples()[0];
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for action in [Action::Play, Action::Download] {
        let (mut responses, request) = flow(sample, true);
        responses.pop();
        let mut f = fixture::setup(responses).await;
        if action == Action::Play {
            assert_eq!(
                f.client
                    .native_stream(&credential, &data::track(), &request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::CapabilityNotSupported
            );
        } else {
            let result = f
                .client
                .native_download(&credential, &data::track(), &request)
                .await
                .unwrap();
            assert!(!result.available && result.url.is_none());
            assert_eq!(result.extensions["content_delivery"], "download_content");
            let shown = serde_json::to_string(&result).unwrap();
            assert!(!shown.contains(sample["ekey"].as_str().unwrap()) && !shown.contains("sycdn"));
        }
        fixture::requests(&mut f, 3).await;
    }
    let (mut responses, _) = flow(sample, true);
    responses.pop();
    let mut f = fixture::setup(responses).await;
    let available = f
        .client
        .native_track_availability(
            &credential,
            "67474",
            &TrackAvailabilityRequest {
                bitrate: 128000,
                account: None,
            },
        )
        .await
        .unwrap();
    assert!(available.playable);
    assert_eq!(available.extensions["content_delivery"], "audio_content");
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_content_cdn_rejections_cannot_invalidate_login_or_retry() {
    let sample = &samples()[0];
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let plain = STANDARD
        .decode(sample["plain_base64"].as_str().unwrap())
        .unwrap();
    for bad in [
        response(401,"application/octet-stream","",b"private"),
        response(302,"application/octet-stream","Location: http://127.0.0.1/never\r\n",b"private"),
        response(206,"application/octet-stream","",&plain),
        response(200,"text/html","",&plain),
        response(200,"audio/mpeg","Content-Encoding: gzip\r\n",&plain),
        response(200,"audio/mpeg","",b"<html>not audio</html>"),
        response(200,"audio/mpeg","",&plain[..plain.len()-1]),
        format!("HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_BYTES+1).into_bytes(),
        b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 99\r\nConnection: close\r\n\r\nx".to_vec(),
    ] {
        let (mut responses, request) = flow(sample,false); *responses.last_mut().unwrap() = bad;
        let mut f = fixture::setup(responses).await;
        let error = f.client.native_audio_content(&credential,&data::track(),&request).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("selected-session"));
        fixture::requests(&mut f,4).await;
    }
}

#[tokio::test]
async fn native_content_chunked_transfer_and_total_budget_are_bounded() {
    let sample = &samples()[0];
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let cipher = STANDARD
        .decode(sample["cipher_base64"].as_str().unwrap())
        .unwrap();
    let mut reply = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    for chunk in cipher.chunks(127) {
        reply.extend(format!("{:x}\r\n", chunk.len()).as_bytes());
        reply.extend(chunk);
        reply.extend(b"\r\n");
    }
    reply.extend(b"0\r\n\r\n");
    let (mut responses, request) = flow(sample, true);
    *responses.last_mut().unwrap() = reply;
    let mut f = fixture::setup(responses).await;
    assert_eq!(
        f.client
            .native_audio_content(&credential, &data::track(), &request)
            .await
            .unwrap()
            .bytes,
        STANDARD
            .decode(sample["plain_base64"].as_str().unwrap())
            .unwrap()
    );
    fixture::requests(&mut f, 4).await;
    let control = control();
    control.cancelled.store(true, Ordering::Relaxed);
    assert_eq!(
        control.check().unwrap_err().code,
        ErrorCode::UpstreamTimeout
    );
    let f = fixture::setup(vec![]).await;
    assert_eq!(
        f.client
            .fetch_native_content(
                &input(),
                &data::track(),
                &request,
                Action::Play,
                Duration::ZERO,
                || Ok(())
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamTimeout
    );
}

#[tokio::test]
async fn native_content_download_denial_cannot_reuse_prior_play_authorization() {
    let sample = &samples()[0];
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let (mut replies, request) = flow(sample, true);
    let mut denied = data::rights();
    denied["songs"][0]["payInfo"]["cannotDownload"] = json!(1);
    replies.extend([
        json_response(&json!({"result":"ok"})),
        data::rights_reply(&denied),
    ]);
    let mut f = fixture::setup(replies).await;
    assert!(
        f.client
            .native_audio_content(&credential, &data::track(), &request)
            .await
            .is_ok()
    );
    assert_eq!(
        f.client
            .native_download_content(&credential, &data::track(), &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let calls = fixture::requests(&mut f, 6).await;
    assert_eq!(data::query(&calls[1])["action"], "play");
    assert_eq!(data::query(&calls[5])["action"], "download");
}

#[tokio::test]
async fn native_large_flac_content_allows_master_and_vinyl_with_a_separate_hard_limit() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };
    for quality in [Quality::Standard, Quality::Master, Quality::Vinyl] {
        for declared in [200 * 1024 * 1024, MAX_LARGE_FLAC_BYTES + 1] {
            let (responses, request) = if quality == Quality::Vinyl {
                super::super::vinyl_tests::flow(true)
            } else {
                let sample = if quality == Quality::Master {
                    samples()
                        .into_iter()
                        .find(|s| s["quality"] == "master")
                        .unwrap()
                } else {
                    samples().remove(0)
                };
                flow(&sample, true)
            };
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin =
                Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
            let (headers_sent, received_headers) = oneshot::channel();
            let (release, wait_release) = oneshot::channel();
            let server = tokio::spawn(async move {
                for body in responses.into_iter().take(3) {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    read_headers(&mut socket).await;
                    socket.write_all(&body).await.unwrap();
                }
                let (mut socket, _) = listener.accept().await.unwrap();
                read_headers(&mut socket).await;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                headers_sent.send(()).unwrap();
                let _ = wait_release.await;
                let _ = socket.shutdown().await;
            });
            let mut client = fixture::setup(vec![]).await.client.clone();
            client.web_test_origin = Some(origin);
            let mut task = tokio::spawn(async move {
                client
                    .native_audio_content(
                        &fixture::credential_fixture("42", "selected-session")
                            .caller()
                            .unwrap(),
                        &data::track(),
                        &request,
                    )
                    .await
            });
            tokio::time::timeout(Duration::from_secs(10), received_headers)
                .await
                .unwrap()
                .unwrap();
            if matches!(quality, Quality::Master | Quality::Vinyl)
                && declared <= MAX_LARGE_FLAC_BYTES
            {
                // The accepted large FLAC header must enter body streaming. The
                // server intentionally sends no bytes, so it stays pending.
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), &mut task)
                        .await
                        .is_err()
                );
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                let error = tokio::time::timeout(Duration::from_secs(2), &mut task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(error.code, ErrorCode::UpstreamError);
            }
            let _ = release.send(());
            server.await.unwrap();
        }
    }
    async fn read_headers(socket: &mut tokio::net::TcpStream) {
        let mut request = Vec::new();
        loop {
            let mut buf = [0; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buf[..n]);
            assert!(request.len() < 8192);
            if request.windows(4).any(|s| s == b"\r\n\r\n") {
                break;
            }
        }
    }
}

#[test]
fn dtsx_mmp4_checks_sample_map_and_ftoc_framing_without_decoding_audio() {
    let valid = super::dtsx::synthetic_fixture();
    assert_eq!(
        format::inspect(&valid, "mmp4", &control()).unwrap(),
        ("audio/mp4", "mp4")
    );
    assert!(format::inspect(&valid, "aac", &control()).is_err());
    for size in [0, 1, 7, valid.len() - 1, valid.len() / 2] {
        assert!(format::inspect(&valid[..size], "mmp4", &control()).is_err());
    }

    let mut bad_crc = valid.clone();
    let first_sync = bad_crc
        .windows(4)
        .position(|bytes| bytes == [0x40, 0x41, 0x1b, 0xf2])
        .unwrap();
    bad_crc[first_sync + 30] ^= 1;
    assert!(format::inspect(&bad_crc, "mmp4", &control()).is_err());

    let mut bad_chunk_offset = valid.clone();
    let stco = bad_chunk_offset
        .windows(4)
        .position(|bytes| bytes == b"stco")
        .unwrap();
    bad_chunk_offset[stco + 12..stco + 16].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(format::inspect(&bad_chunk_offset, "mmp4", &control()).is_err());

    let mut wrong_codec = valid;
    let dtsx = wrong_codec
        .windows(4)
        .position(|bytes| bytes == b"dtsx")
        .unwrap();
    wrong_codec[dtsx..dtsx + 4].copy_from_slice(b"mp4a");
    assert!(format::inspect(&wrong_codec, "mmp4", &control()).is_err());
}
