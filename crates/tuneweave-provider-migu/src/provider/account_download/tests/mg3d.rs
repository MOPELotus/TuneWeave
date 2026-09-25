use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

const FILE_KEY: &str = "0123456789abcdef0123456789abcdef";
const MEDIA: usize = 6;
fn plain() -> Vec<u8> {
    let mut frame = vec![0_u8; 417];
    frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
    frame.repeat(40)
}
fn encrypted() -> Vec<u8> {
    let key = b"CB4E917FFEB2B4A056445F4B3544495E";
    plain()
        .iter()
        .enumerate()
        .map(|(i, v)| (*v).wrapping_add(key[i % 32]))
        .collect()
}
fn media_frame(bytes: &[u8], extra: &str) -> Vec<u8> {
    let mut result=format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",bytes.len()).into_bytes();
    result.extend_from_slice(bytes);
    result
}
fn grant() -> serde_json::Value {
    json!({"code":"000000","data":{"contentId":"123","copyrightId":"6005971HBUU","formatId":"020007","encryptionType":"0","fileKey":FILE_KEY,"size":plain().len(),"suffix":"mp3","url":"https://dlsdownfree.nf.migu.cn/wlansst/song?pars=download-fixture"}})
}
fn responses() -> Vec<Vec<u8>> {
    let mut base = frames();
    base[4] = base[4].replace("03:32", "00:01"); // Equal length preserves HTTP fixture length.
    base[5] = reply(grant(), None);
    let mut result = base.into_iter().map(String::into_bytes).collect::<Vec<_>>();
    result.insert(MEDIA, media_frame(&encrypted(), ""));
    result
}
struct Harness {
    provider: MiguProvider,
    wire: tokio::task::JoinHandle<Vec<String>>,
    seen: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
}
async fn binary_server(responses: Vec<Vec<u8>>, gate: Option<usize>) -> Harness {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (seen_tx, seen) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let wire = tokio::spawn(async move {
        let mut wire = Vec::new();
        let mut signals = Some((seen_tx, release_rx));
        for (index, response) in responses.into_iter().enumerate() {
            let request = tokio::time::timeout(Duration::from_secs(5), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                }
                if gate == Some(index) {
                    let (seen, release) = signals.take().unwrap();
                    seen.send(()).unwrap();
                    release.await.unwrap();
                }
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
                String::from_utf8(bytes).unwrap()
            })
            .await
            .expect("MG3D fixture request timed out");
            wire.push(request);
        }
        wire
    });
    Harness {
        provider: MiguProvider::from_client(
            MiguClient::test_client().with_catalog_test_origin(origin),
        ),
        wire,
        seen,
        release,
    }
}

#[tokio::test]
async fn mg3d_content_authorizes_isolated_accounts_and_never_sends_keys_to_cdn() {
    for mode in ["default", "named", "caller"] {
        let mut h = binary_server(responses(), None).await;
        let (store, original, alias) = setup(&mut h.provider, mode);
        let result = h
            .provider
            .audio_download_content(&track(), &request(alias))
            .await
            .unwrap();
        assert_eq!(result.bytes, plain());
        assert_eq!(result.track_ref, track().resource_ref);
        assert_eq!(result.content_type, "audio/mpeg");
        assert_eq!(result.filename, "123.mp3");
        assert!(result.trial.is_none());
        for secret in [FILE_KEY, "native-token-fixture", "download-fixture", "pacm"] {
            assert!(!format!("{result:?}").contains(secret));
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            let update = h.provider.take_response_credential().unwrap().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap().token(),
                "final-pacm"
            );
            assert!(!update.secret().contains(FILE_KEY));
            assert!(h.provider.take_response_credential().unwrap().is_none());
        } else {
            assert_eq!(read(&store, alias).token(), "final-pacm");
        }
        let wire = h.wire.await.unwrap();
        assert_eq!(wire.len(), 8);
        assert!(wire[5].starts_with("GET /MIGUM2.0/strategy/download-url/by-songid/v1.0?contentId=123&formatType=PQ&songId=456 "));
        assert!(wire[MEDIA].starts_with("GET /wlansst/song?pars=download-fixture "));
        for forbidden in [
            FILE_KEY,
            "pacmtoken",
            "cookie:",
            "token:",
            "ce:",
            "sign:",
            "usessionid",
            "authorization:",
            "range:",
        ] {
            assert!(
                !wire[MEDIA]
                    .to_lowercase()
                    .contains(&forbidden.to_lowercase()),
                "{forbidden}"
            );
        }
        assert!(
            wire.iter().all(|r| !r.contains("/listen")
                && !r.contains("can-listen")
                && !r.contains(".mgm"))
        );
    }
}

#[tokio::test]
async fn mg3d_grant_requires_keys_full_download_identity_format_and_safe_url() {
    for (field, value) in [
        ("encryptionType", json!("2")),
        ("fileKey", json!("short")),
        ("fileKey", json!("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")),
        ("contentId", json!("222")),
        ("copyrightId", json!("wrong")),
        ("formatId", json!("wrong")),
        ("suffix", json!("flac")),
        ("auditionsLength", json!(60)),
        ("size", json!(0)),
        ("size", json!(64 * 1024 * 1024 + 1)),
        (
            "url",
            json!("http://dlsdownfree.nf.migu.cn/wlansst?pars=fixture"),
        ),
        ("url", json!("https://evil.example/wlansst?pars=fixture")),
        (
            "url",
            json!(format!(
                "https://dlsdownfree.nf.migu.cn/wlansst?pars={FILE_KEY}"
            )),
        ),
        (
            "url",
            json!("https://dlsdownfree.nf.migu.cn/wlansst?pars=verified-pacm"),
        ),
    ] {
        let mut frames = responses();
        let mut body = grant();
        body["data"][field] = value;
        frames[5] = reply(body, None).into_bytes();
        frames.truncate(6);
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        let failure = h
            .provider
            .audio_download_content(&track(), &request(alias))
            .await
            .unwrap_err();
        assert!(!format!("{failure:?}").contains(FILE_KEY));
        assert_eq!(h.wire.await.unwrap().len(), 6, "{field}");
    }
}

#[tokio::test]
async fn mg3d_denied_or_corrupt_transfers_return_no_plaintext_and_never_retry() {
    let mut corrupt = encrypted();
    corrupt[0] ^= 1;
    let truncated = encrypted()[..encrypted().len() - 1].to_vec();
    for response in [
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        b"HTTP/1.1 206 Partial Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        media_frame(&encrypted(),"Content-Range: bytes 0-16679/16680\r\n"),
        media_frame(&encrypted(),"Content-Encoding: gzip\r\n"),
        media_frame(&corrupt,""),media_frame(&truncated,""),
        media_frame(&vec![b'x';plain().len()],""),
    ] {
        let mut frames=responses();frames[MEDIA]=response;frames.truncate(MEDIA+1);
        let mut h=binary_server(frames,None).await;let(store,original,alias)=setup(&mut h.provider,"caller");
        let failure=h.provider.audio_download_content(&track(),&request(alias)).await.unwrap_err();assert!(!format!("{failure:?}").contains(FILE_KEY));
        assert_eq!(read(&store,alias),original);assert_eq!(h.wire.await.unwrap().len(),7);
    }
}

#[tokio::test]
async fn mg3d_identity_or_generation_changes_never_deliver_content() {
    for at in [3, 7] {
        let mut frames = responses();
        frames[at] = if at == 3 {
            validation("222")
        } else {
            profile("222", "")
        }
        .into_bytes();
        frames.truncate(at + 1);
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "caller");
        assert!(
            h.provider
                .audio_download_content(&track(), &request(alias))
                .await
                .is_err()
        );
        if at == 7 {
            assert!(h.provider.take_response_credential().unwrap().is_none());
        }
        assert_eq!(h.wire.await.unwrap().len(), at + 1);
    }
    for at in [5, MEDIA, 7] {
        let mut frames = responses();
        frames.truncate(at + 1);
        let mut h = binary_server(frames, Some(at)).await;
        let (store, _, alias) = setup(&mut h.provider, "named");
        let provider = h.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .audio_download_content(&track(), &request(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        let next = MiguCredential::verified("111".into(), "new-login-pacm".into()).unwrap();
        store.put(&stored(alias, &next)).unwrap();
        h.release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&store, alias), next);
        assert_eq!(h.wire.await.unwrap().len(), at + 1);
    }
}

#[tokio::test]
async fn mg3d_cancel_discards_caller_rotation_at_download_boundaries() {
    for at in [5, MEDIA, 7] {
        let mut frames = responses();
        frames.truncate(at + 1);
        let mut h = binary_server(frames, Some(at)).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let provider = h.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .audio_download_content(&track(), &request(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(h.provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, alias), original);
        h.wire.abort();
    }
}

#[tokio::test]
async fn mg3d_unselected_accounts_and_unsupported_quality_fail_without_network() {
    let mut h = binary_server(vec![], None).await;
    let (_, _, alias) = setup(&mut h.provider, "named");
    for req in [
        StreamRequest::default(),
        StreamRequest {
            quality: Quality::Dolby,
            ..request(alias)
        },
        request("missing"),
    ] {
        assert!(
            h.provider
                .audio_download_content(&track(), &req)
                .await
                .is_err()
        );
    }
    assert!(h.wire.await.unwrap().is_empty());
}

#[tokio::test]
async fn mg3d_hq_and_auto_keep_the_authorized_mp3_rendition() {
    for quality in [Quality::High, Quality::Auto] {
        let mut frames = responses();
        frames[4] = String::from_utf8(frames[4].clone())
            .unwrap()
            .replace("PQ", "HQ")
            .replace("020007", "020009")
            .into_bytes();
        let mut frame = vec![0_u8; 1044];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0xe0, 0]);
        let clear = frame.repeat(40);
        let key = b"CB4E917FFEB2B4A056445F4B3544495E";
        let encrypted: Vec<u8> = clear
            .iter()
            .enumerate()
            .map(|(i, v)| (*v).wrapping_add(key[i % 32]))
            .collect();
        let mut body = grant();
        body["data"]["formatId"] = json!("020009");
        body["data"]["size"] = json!(clear.len());
        frames[5] = reply(body, None).into_bytes();
        frames[6] = media_frame(&encrypted, "");
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        let result = h
            .provider
            .audio_download_content(
                &track(),
                &StreamRequest {
                    quality,
                    ..request(alias)
                },
            )
            .await
            .unwrap();
        assert_eq!(result.bytes, clear);
        let wire = h.wire.await.unwrap();
        assert!(wire[5].contains("formatType=HQ"));
        assert_eq!(wire.len(), 8);
    }
}

#[tokio::test]
async fn mg3d_url_download_does_not_export_encrypted_resources() {
    let mut frames = responses();
    frames.truncate(6);
    let mut h = binary_server(frames, None).await;
    let (_, _, alias) = setup(&mut h.provider, "named");
    let failure = h
        .provider
        .download(&track(), &request(alias))
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::PermissionDenied);
    assert_eq!(h.wire.await.unwrap().len(), 6);
}

mod aliases;
mod clear;
mod encryption_flag;
mod flac;
