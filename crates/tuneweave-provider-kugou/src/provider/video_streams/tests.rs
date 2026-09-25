use super::*;
use crate::provider::session::tests::{Store, credential, raw, server};
use serde_json::Value;
use tuneweave_core::VideoResourceKind;

mod native;

fn hash(id: u64) -> String {
    format!("{id:032X}")
}
fn video(id: u64) -> Value {
    json!({"video_id":id,"video_name":"Video","timelength":123456,"is_publish":1,"deleted":0,
        "sd_hash":hash(id),"sd_height":432,"sd_width":768,"sd_filesize":1000,"sd_bitrate":800})
}
fn details(ids: &[u64]) -> Value {
    json!({"status":1,"error_code":0,"data":ids.iter().copied().map(video).collect::<Vec<_>>()})
}
fn permission(id: u64) -> Value {
    json!({"hash":hash(id),"id":id,"status":1,"privilege":0,"pay_type":0,"fail_process":0,"info":{"filesize":1000,"bitrate":800}})
}
fn rights(items: Vec<Value>) -> Value {
    json!({"status":1,"error_code":0,"data":items})
}
fn tracker(id: u64) -> Value {
    json!({"status":1,"privileges":{hash(id).to_lowercase():0},"data":{hash(id).to_lowercase():{
        "filesize":"1000","downurl":format!("https://mvwebfs.tx.kugou.com/{id}?auth=test%2Bvalue"),
        "backupdownurl":[format!("https://mvwebfs.ali.kugou.com/{id}?auth=backup")]}}})
}
fn request() -> VideoStreamRequest {
    VideoStreamRequest::new(VideoResourceKind::Mv, 480)
}
fn params(request: &str) -> BTreeMap<String, String> {
    let uri = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    url::Url::parse(&format!("http://localhost{uri}"))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test]
async fn anonymous_streams_bind_rights_and_tracker_signatures_and_preserve_batch_references() {
    let mut f = server(vec![
        raw(details(&[2, 1])).into(),
        raw(rights(vec![permission(1)])).into(),
        raw(tracker(1))
            .replace("application/json", "application/octet-stream")
            .into(),
        raw(rights(vec![permission(2)])).into(),
        raw(tracker(2)).into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    let old = credential("999", "never-send-account-token");
    store.put(&old.stored("default").unwrap()).unwrap();
    f.provider.credential_store = Some(store.clone());
    let ids = vec!["mv:1".into(), "2".into(), "1".into(), "mv:2".into()];
    let result = f.provider.video_streams(&ids, &request()).await.unwrap();
    for (s, id) in result.iter().zip(&ids) {
        assert_eq!(s.video_ref.id(), id);
        assert!(s.available);
        assert_eq!(s.actual_resolution, Some(432));
        assert_eq!(s.width, Some(768));
        assert_eq!(s.size, Some(1000));
        assert_eq!(s.duration_ms, Some(123456));
        assert_eq!(s.requested_resolution, 480);
        assert_eq!(s.backup_urls.len(), 1);
        assert_eq!(s.expires_at, None);
        assert_eq!(s.format, None);
        assert_eq!(s.codec, None);
        assert!(s.headers.is_empty());
    }
    assert_eq!(result[0].url, result[2].url);
    let reqs = f.requests.await.unwrap();
    assert_eq!(reqs.len(), 5);
    for r in &reqs[1..] {
        let q = params(r);
        let mut unsigned = q.clone();
        let signature = unsigned.remove("signature").unwrap();
        let bytes = r.split_once("\r\n\r\n").unwrap().1.as_bytes();
        assert_eq!(
            signature,
            crate::signing::android_signature(
                &unsigned
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.clone()))
                    .collect(),
                bytes
            )
        );
        assert_eq!(q["userid"], "0");
        assert_eq!(q["token"], "");
        assert_eq!(q["dfid"], "-");
        assert!(!r.contains("never-send-account-token"));
        assert!(!r.to_lowercase().contains("cookie:"));
        assert!(!r.to_lowercase().contains("authorization:"));
        if r.starts_with("GET") {
            assert!(r.starts_with("GET /v2/interface/index?"));
            assert_eq!(q["ssl"], "1");
            assert_eq!(q["pid"], "1");
            assert_eq!(q["backupdomain"], "1");
            assert_eq!(q["ext"], "mp4");
            use md5::{Digest, Md5};
            assert_eq!(
                q["key"],
                hex::encode(Md5::digest(
                    format!(
                        "{}57ae12eb6890223e355ccfcb74edf70d1005{}0",
                        q["hash"], q["mid"]
                    )
                    .as_bytes()
                ))
            );
            assert!(r.to_lowercase().contains("x-router: trackermv.kugou.com"));
        } else {
            assert!(r.starts_with("POST /v1/get_video_privilege?"));
            let b: Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(b["behavior"], "play");
            assert_eq!(b["userid"], 0);
            assert_eq!(b["vip"], 0);
            assert_eq!(b["token"], "");
            assert_eq!(b["mid"], q["mid"]);
            assert!(r.to_lowercase().contains("x-router: media.store.kugou.com"));
        }
    }
    assert_eq!(
        store.values.lock().unwrap()["default"],
        old.stored("default").unwrap()
    );
}

#[tokio::test]
async fn selects_an_authorized_lower_resolution_and_never_requests_a_denied_hash() {
    let mut v = video(1);
    v["hd_hash"] = json!(hash(2));
    v["hd_height"] = json!(720);
    v["hd_width"] = json!(1280);
    v["hd_filesize"] = json!(1000);
    v["hd_bitrate"] = json!(800);
    let mut denied = permission(2);
    denied["id"] = json!(1);
    denied["status"] = json!(0);
    denied["privilege"] = json!(10);
    denied["info"] = Value::Null;
    let f = server(vec![
        raw(json!({"status":1,"error_code":0,"data":[v]})).into(),
        raw(rights(vec![permission(1), denied])).into(),
        raw(tracker(1)).into(),
    ])
    .await;
    let s = f
        .provider
        .video_stream("1", &VideoStreamRequest::new(VideoResourceKind::Mv, 1080))
        .await
        .unwrap();
    assert!(s.available);
    assert_eq!(s.actual_resolution, Some(432));
    let reqs = f.requests.await.unwrap();
    assert_eq!(reqs.len(), 3);
    assert_eq!(params(&reqs[2])["hash"], hash(1));
}

#[tokio::test]
async fn missing_assets_and_denied_rights_do_not_request_trackers_or_claim_availability() {
    let f = server(vec![
        raw(json!({"status":1,"error_code":0,"data":[{"video_id":1,"video_name":"No resources"}]}))
            .into(),
    ])
    .await;
    let s = f.provider.video_stream("1", &request()).await.unwrap();
    assert!(!s.available);
    assert_eq!(s.actual_resolution, None);
    assert_eq!(f.requests.await.unwrap().len(), 1);
    for (status, fail) in [(0, 0), (1, 1)] {
        let mut p = permission(1);
        p["status"] = json!(status);
        p["fail_process"] = json!(fail);
        let f = server(vec![raw(details(&[1])).into(), raw(rights(vec![p])).into()]).await;
        let s = f.provider.video_stream("1", &request()).await.unwrap();
        assert!(!s.available);
        assert!(s.url.is_none());
        assert_eq!(s.size, None);
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn unknown_dimensions_are_playable_without_fabricating_resolution_or_container() {
    let mut v = video(1);
    v["sd_width"] = json!("");
    v["sd_height"] = json!("");
    let f = server(vec![
        raw(json!({"status":1,"error_code":0,"data":[v]})).into(),
        raw(rights(vec![permission(1)])).into(),
        raw(tracker(1)).into(),
    ])
    .await;
    let s = f.provider.video_stream("1", &request()).await.unwrap();
    assert!(s.available);
    assert_eq!(s.actual_resolution, None);
    assert_eq!(s.width, None);
    assert_eq!(s.format, None);
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn duplicated_catalogue_hashes_are_fetched_once_only_when_their_metadata_agrees() {
    for height in [432, 540] {
        let mut v = video(1);
        v["mkv_sd_hash"] = json!(hash(1));
        v["mkv_sd_height"] = json!(height);
        v["mkv_sd_width"] = json!(768);
        v["mkv_sd_filesize"] = json!(1000);
        v["mkv_sd_bitrate"] = json!(800);
        let mut frames = vec![raw(json!({"status":1,"error_code":0,"data":[v]})).into()];
        if height == 432 {
            frames.extend([
                raw(rights(vec![permission(1)])).into(),
                raw(tracker(1)).into(),
            ]);
        }
        let f = server(frames).await;
        let result = f.provider.video_stream("1", &request()).await;
        if height == 432 {
            let s = result.unwrap();
            assert!(s.available);
            assert_eq!(s.extensions["selected_resource"]["source_key"], "sd");
            let reqs = f.requests.await.unwrap();
            assert_eq!(reqs.len(), 3);
            let b: Value = serde_json::from_str(reqs[1].split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(b["resource"].as_array().unwrap().len(), 1);
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::UpstreamError);
            assert_eq!(f.requests.await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn late_tracker_failure_and_false_positive_permissions_never_return_partial_batches() {
    let f = server(vec![
        raw(details(&[1, 2])).into(),
        raw(rights(vec![permission(1)])).into(),
        raw(tracker(1)).into(),
        raw(rights(vec![permission(2)])).into(),
        raw(json!({"status":0,"errcode":40002})).into(),
    ])
    .await;
    assert_eq!(
        f.provider
            .video_streams(&["1".into(), "2".into()], &request())
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(f.requests.await.unwrap().len(), 5);
    let mut p = permission(1);
    p["id"] = json!(0);
    p["info"] = json!({"filesize":0,"bitrate":0});
    let f = server(vec![raw(details(&[1])).into(), raw(rights(vec![p])).into()]).await;
    assert_eq!(
        f.provider
            .video_stream("1", &request())
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(f.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn playback_transport_enforces_response_limits_mime_ssa_and_no_redirects() {
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: https://evil.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_owned(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_owned(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSSA-CODE: 123\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_owned(),
    ] {
        let f=server(vec![raw(details(&[1])).into(),response.into()]).await;
        assert!(f.provider.video_stream("1",&request()).await.is_err());assert_eq!(f.requests.await.unwrap().len(),2);
    }
}

#[tokio::test]
async fn tracker_octet_stream_exception_does_not_accept_html_or_non_json_or_relax_privileges() {
    let f = server(vec![
        raw(details(&[1])).into(),
        raw(rights(vec![permission(1)]))
            .replace("application/json", "application/octet-stream")
            .into(),
    ])
    .await;
    assert_eq!(
        f.provider
            .video_stream("1", &request())
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(f.requests.await.unwrap().len(), 2);
    for response in [raw(tracker(1)).replace("application/json","text/html"),"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 5\r\nConnection: close\r\n\r\n<html".into()] {
        let f=server(vec![raw(details(&[1])).into(),raw(rights(vec![permission(1)])).into(),response.into()]).await;
        assert_eq!(f.provider.video_stream("1",&request()).await.unwrap_err().code,ErrorCode::UpstreamError);
        assert_eq!(f.requests.await.unwrap().len(),3);
    }
}

#[tokio::test]
async fn stream_validation_rejects_missing_accounts_and_invalid_inputs_before_network() {
    let f = server(vec![]).await;
    for kind in [VideoResourceKind::Mv, VideoResourceKind::Video] {
        let r = VideoStreamRequest::new(kind, 1080);
        for id in [
            "",
            "0",
            "01",
            "hash",
            if kind == VideoResourceKind::Mv {
                "video:1"
            } else {
                "mv:1"
            },
        ] {
            assert_eq!(
                f.provider.video_stream(id, &r).await.unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        for ids in [vec![], vec!["1".into(); 101]] {
            assert_eq!(
                f.provider.video_streams(&ids, &r).await.unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        let mut invalid = r.clone();
        invalid.account = Some("default".into());
        assert_eq!(
            f.provider
                .video_stream("1", &invalid)
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        invalid = r.clone();
        invalid.resolution = 0;
        assert_eq!(
            f.provider
                .video_stream("1", &invalid)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let p = f
            .provider
            .caller_scope(&credential("999", "private").caller().unwrap())
            .unwrap();
        assert_eq!(
            p.video_stream("not-an-id", &r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(f.requests.await.unwrap().len(), 0);
}

#[tokio::test]
#[ignore = "opt-in official anonymous video streams and bounded HTTPS range verification"]
async fn official_anonymous_video_streams_match_metadata_and_return_readable_https_media() {
    let p = KugouProvider::new(KugouConfig::default()).unwrap();
    for (id, kind, resolution) in [
        ("mv:17737761", VideoResourceKind::Mv, 1080),
        ("video:17781851", VideoResourceKind::Video, 480),
    ] {
        let d = p.video(id, &VideoDetailRequest::new(kind)).await.unwrap();
        let s = p
            .video_stream(id, &VideoStreamRequest::new(kind, resolution))
            .await
            .unwrap();
        assert!(s.available, "no authorized anonymous stream");
        assert_eq!(s.video_ref.id(), id);
        assert_eq!(s.duration_ms, d.video.duration_ms);
        let size = s.size.expect("verified file size");
        for u in s.url.iter().chain(&s.backup_urls) {
            assert!(u.starts_with("https://"));
            let mut r = p
                .client
                .http
                .get(u)
                .header("Range", "bytes=0-4095")
                .send()
                .await
                .map_err(|e| e.without_url())
                .expect("HTTPS range request");
            assert_eq!(r.status(), reqwest::StatusCode::PARTIAL_CONTENT);
            assert_eq!(
                r.headers().get("content-range").unwrap().to_str().unwrap(),
                format!("bytes 0-4095/{size}")
            );
            let mut prefix = Vec::new();
            while prefix.len() < 4096 {
                let chunk = r.chunk().await.unwrap().expect("media bytes");
                prefix.extend_from_slice(&chunk[..chunk.len().min(4096 - prefix.len())]);
            }
            assert!(
                prefix.get(4..8) == Some(b"ftyp") || prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
                "recognized media container header"
            );
        }
    }
}
