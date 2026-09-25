use super::*;
use crate::provider::session::tests::{Store, credential, raw, server};
use serde_json::Value;
use std::collections::BTreeMap;
use tuneweave_core::{VideoDetailRequest, VideoKind};

fn artist(total: Option<u64>) -> Value {
    json!({"status":1,"error_code":0,"data":{"author_id":35,"author_name":"Query singer","mv_count":total}})
}
fn list(page: u32, total: u64) -> Value {
    let start = u64::from(page - 1) * 30;
    json!({"status":1,"error_code":0,"errcode":0,"total":total,"extra":{"page_total":total},
        "data":(start..total.min(start+30)).map(|n| json!({
            "video_id":n+1,"video_name":format!("Video {}",n+1),"timelength":140783,
            "album_audio_id":n+81,"audio_id":n+51,"user_id":"99","author_name":"Uploader"
        })).collect::<Vec<_>>()})
}
fn details(ids: impl IntoIterator<Item = u64>) -> Value {
    json!({"status":1,"error_code":0,"errcode":0,"data":ids.into_iter().map(|id| json!({
        "video_id":id.to_string(),"video_name":format!("Video {id}"),"timelength":"140783",
        "album_audio_id":id+80,"audio_id":id+50,"user_id":"99","author_name":"Uploader",
        "authors":[{"author_id":36,"author_name":"Actual credited singer"}],
        "cover":"https://imge.kugou.com/mvpic/video.jpg","audio_timelength":"269000"
    })).collect::<Vec<_>>()})
}
fn request(limit: u32, offset: u32, kind: VideoKind) -> ArtistVideoListRequest {
    let mut r = ArtistVideoListRequest::new(limit, offset);
    r.kind = kind;
    r
}
fn params(r: &str) -> BTreeMap<String, String> {
    let uri = r.lines().next().unwrap().split_whitespace().nth(1).unwrap();
    url::Url::parse(&format!("http://localhost{uri}"))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test]
async fn artist_video_all_and_mv_use_the_same_official_pool_without_fabricating_artist_creators() {
    for kind in [VideoKind::All, VideoKind::Mv] {
        let mut f = server(vec![
            raw(artist(Some(35))).into(),
            raw(list(1, 35)).into(),
            raw(list(2, 35)).into(),
            raw(details(29..=33)).into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let old = credential("999", "account-secret");
        store.put(&old.stored("default").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let p = f
            .provider
            .artist_videos("35", &request(5, 28, kind))
            .await
            .unwrap();
        assert_eq!(p.items.len(), 5);
        assert_eq!(p.pagination.next_offset, Some(33));
        assert_eq!(p.pagination.total, Some(35));
        assert_eq!(p.pagination.extensions["kind"], json!(kind));
        assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 2);
        assert_eq!(p.pagination.extensions["detail_batches_fetched"], 1);
        assert_eq!(p.items[0].resource_ref.to_string(), "kugou:mv:29");
        assert_eq!(p.items[0].duration_ms, Some(140783));
        assert_eq!(p.items[0].extensions["audio_duration_ms"], 269000);
        assert_eq!(
            p.items[0].creators[0].resource_ref.as_ref().unwrap().id(),
            "36"
        );
        assert_eq!(p.items[0].extensions["uploader"]["id"], "99");
        assert_eq!(p.items[0].extensions["uploader"]["name"], "Uploader");
        assert_eq!(p.items[0].extensions["catalogue_artist_id"], "35");
        assert_eq!(p.items[0].extensions["artist_video_position"], 28);
        assert_eq!(p.items[4].extensions["artist_video_position"], 32);
        let reqs = f.requests.await.unwrap();
        assert_eq!(reqs.len(), 4);
        for (index, r) in reqs[1..3].iter().enumerate() {
            assert!(r.starts_with("GET /kmr/v1/author/videos?"));
            let mut p = params(r);
            let signature = p.remove("signature").unwrap();
            assert_eq!(
                signature,
                crate::signing::android_signature(
                    &p.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
                    b""
                )
            );
            assert_eq!(p["page"], (index + 1).to_string());
            assert_eq!(p["pagesize"], "30");
            assert_eq!(p["author_id"], "35");
            assert_eq!(p["tag_idx"], "");
            assert_eq!(p["is_fanmade"], "");
            assert_eq!(p["appid"], "1005");
            assert_eq!(p["clientver"], "20489");
            assert_eq!(p["token"], "");
            assert_eq!(p["userid"], "0");
            assert!(!r.to_lowercase().contains("kg-tid:"));
            assert!(!r.to_lowercase().contains("cookie:"));
            assert!(!r.contains("account-secret"));
            assert!(!r.to_lowercase().contains("authorization:"));
        }
        assert_eq!(params(&reqs[1])["mid"], params(&reqs[2])["mid"]);
        assert_eq!(
            store.values.lock().unwrap().get("default").unwrap(),
            &old.stored("default").unwrap()
        );
    }
}

#[tokio::test]
async fn artist_video_window_uses_at_most_five_pages_and_enriches_only_selected_items() {
    let mut replies = vec![raw(artist(Some(150))).into()];
    replies.extend((1..=5).map(|p| raw(list(p, 150)).into()));
    replies.extend(
        (30..=129)
            .collect::<Vec<_>>()
            .chunks(20)
            .map(|ids| raw(details(ids.iter().copied())).into()),
    );
    let f = server(replies).await;
    let p = f
        .provider
        .artist_videos("35", &request(100, 29, VideoKind::All))
        .await
        .unwrap();
    assert_eq!(p.items.len(), 100);
    assert_eq!(p.items[0].id, "30");
    assert_eq!(p.items[99].id, "129");
    assert_eq!(p.pagination.next_offset, Some(129));
    assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 5);
    assert_eq!(p.pagination.extensions["detail_batches_fetched"], 5);
    let reqs = f.requests.await.unwrap();
    assert_eq!(reqs.len(), 11);
    for (index, r) in reqs[6..].iter().enumerate() {
        let b: Value = serde_json::from_str(r.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(b["data"].as_array().unwrap().len(), 20);
        assert_eq!(b["data"][0]["video_id"], (30 + index * 20).to_string());
    }
}

#[tokio::test]
async fn artist_video_tail_empty_and_out_of_range_keep_upstream_totals_without_unneeded_enrichment()
{
    for (total, offset, count) in [(35, 33, 2), (35, 35, 0), (35, 60, 0), (0, 0, 0)] {
        let mut replies = vec![
            raw(artist(None)).into(),
            raw(list(offset / 30 + 1, total)).into(),
        ];
        if count > 0 {
            replies.push(raw(details(u64::from(offset) + 1..=total)).into());
        }
        let f = server(replies).await;
        let p = f
            .provider
            .artist_videos("35", &request(10, offset, VideoKind::Mv))
            .await
            .unwrap();
        assert_eq!(p.items.len(), count);
        assert_eq!(p.pagination.total, Some(total));
        assert!(!p.pagination.has_more);
        assert_eq!(p.pagination.next_offset, None);
        assert_eq!(
            f.requests.await.unwrap().len(),
            if count > 0 { 3 } else { 2 }
        );
    }
}

#[tokio::test]
async fn artist_video_windows_reject_count_and_duplicate_conflicts_before_detail_reads() {
    for mode in 0..3 {
        let mut second = list(2, 35);
        let expected = if mode == 0 { Some(36) } else { Some(35) };
        if mode == 1 {
            second = list(2, 36);
        }
        if mode == 2 {
            second["data"][0]["video_id"] = json!(1);
        }
        let mut responses = vec![raw(artist(expected)).into(), raw(list(1, 35)).into()];
        if mode != 0 {
            responses.push(raw(second).into());
        }
        let f = server(responses).await;
        assert_eq!(
            f.provider
                .artist_videos("35", &request(5, 28, VideoKind::All))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(
            f.requests.await.unwrap().len(),
            if mode == 0 { 2 } else { 3 }
        );
    }
}

#[tokio::test]
async fn artist_video_enrichment_rejects_wrong_video_audio_duration_and_uploader_without_partial_results()
 {
    for (key, value) in [
        ("video_id", json!(2)),
        ("album_audio_id", json!(999)),
        ("audio_id", json!(999)),
        ("timelength", json!(269000)),
        ("user_id", json!(100)),
    ] {
        let mut d = details([1]);
        d["data"][0][key] = value;
        let f = server(vec![
            raw(artist(Some(1))).into(),
            raw(list(1, 1)).into(),
            raw(d).into(),
        ])
        .await;
        assert_eq!(
            f.provider
                .artist_videos("35", &request(1, 0, VideoKind::All))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "{key}"
        );
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let f = server(vec![
        raw(artist(Some(1))).into(),
        raw(list(1, 1)).into(),
        raw(json!({"status":1,"error_code":0,"data":[{}]})).into(),
    ])
    .await;
    assert_eq!(
        f.provider
            .artist_videos("35", &request(1, 0, VideoKind::All))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn artist_video_transport_keeps_openapi_mime_challenge_and_redirect_boundaries() {
    let good = raw(list(1, 1));
    for bad in [
        good.replace("Content-Type: application/json", "Content-Type: text/html"),
        good.replacen("\r\n", "\r\nssa-code: challenge\r\n", 1),
        "HTTP/1.1 302 Found\r\nLocation: https://untrusted.invalid/\r\nContent-Length: 0\r\n\r\n"
            .into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\n\r\n"
            .into(),
        raw(json!({"status":0,"error_code":10,"data":[],"errmsg":"private"})),
    ] {
        let f = server(vec![raw(artist(Some(1))).into(), bad.into()]).await;
        let e = f
            .provider
            .artist_videos("35", &request(1, 0, VideoKind::All))
            .await
            .unwrap_err();
        assert!(!format!("{e:?}").contains("private"));
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn artist_video_validation_rejects_accounts_cursors_sort_and_invalid_ranges_before_network() {
    let f = server(vec![]).await;
    let base = request(1, 0, VideoKind::All);
    for id in [
        "",
        "0",
        "01",
        "+1",
        " 35",
        "video:35",
        "18446744073709551616",
    ] {
        assert_eq!(
            f.provider.artist_videos(id, &base).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for mode in 0..6 {
        let mut r = base.clone();
        match mode {
            0 => r.account = Some("named".into()),
            1 => r.cursor = Some("next".into()),
            2 => r.order = Some("hot".into()),
            3 => r.limit = 0,
            4 => r.limit = 101,
            5 => r.offset = u32::MAX,
            _ => unreachable!(),
        }
        assert_eq!(
            f.provider.artist_videos("35", &r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let scoped = f
        .provider
        .caller_scope(&credential("9", "caller-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        scoped.artist_videos("35", &base).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "uses live official anonymous artist video catalogue and video details"]
async fn live_artist_video_all_mv_cross_page_upload_and_tail_are_consistent_with_details() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    let mut all_ids = None;
    for kind in [VideoKind::All, VideoKind::Mv] {
        let p = provider
            .artist_videos("3520", &request(3, 29, kind))
            .await
            .unwrap();
        assert_eq!(p.items.len(), 3);
        assert_eq!(p.pagination.next_offset, Some(32));
        let ids = p.items.iter().map(|v| v.id.clone()).collect::<Vec<_>>();
        if let Some(all) = &all_ids {
            assert_eq!(all, &ids);
        } else {
            all_ids = Some(ids.clone());
        }
        let d = provider
            .videos(&ids, &VideoDetailRequest::new(VideoResourceKind::Mv))
            .await
            .unwrap();
        for (item, detail) in p.items.iter().zip(d) {
            assert_eq!(item.id, detail.video.id);
            assert_eq!(item.duration_ms, detail.video.duration_ms);
            assert_eq!(item.creators, detail.video.creators);
        }
    }
    let first = provider
        .artist_videos("3520", &request(1, 0, VideoKind::Mv))
        .await
        .unwrap();
    assert!(!first.items[0].creators.is_empty());
    assert!(first.items[0].extensions.contains_key("uploader"));
    let total = u32::try_from(first.pagination.total.unwrap()).unwrap();
    assert!(total > 30);
    let tail = provider
        .artist_videos("3520", &request(10, total - 2, VideoKind::All))
        .await
        .unwrap();
    assert_eq!(tail.items.len(), 2);
    assert!(!tail.pagination.has_more);
    assert_eq!(tail.pagination.total, Some(u64::from(total)));
    let empty = provider
        .artist_videos("3520", &request(10, total + 30, VideoKind::All))
        .await
        .unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.pagination.total, Some(u64::from(total)));
}
