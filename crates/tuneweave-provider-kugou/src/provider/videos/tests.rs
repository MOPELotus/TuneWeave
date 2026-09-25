use super::*;
use crate::provider::session::tests::{Store, credential, raw, server};
use serde_json::Value;
use tuneweave_core::{SearchItem, VideoSearchFilters};

fn video(id: u64) -> Value {
    json!({"video_id":id.to_string(),"video_name":format!("Video {id}"),
        "timelength":"123456","audio_timelength":"120000","audio_id":id+50,"album_audio_id":id+80,
        "hdpic":"https://imge.kugou.com/mvhdpic/{size}/date/cover.jpg",
        "authors":[{"author_id":35,"author_name":"Singer"}],"is_publish":1,"deleted":0})
}
fn details(ids: impl IntoIterator<Item = u64>) -> Value {
    json!({"status":1,"error_code":0,"errcode":0,"data":ids.into_iter().map(video).collect::<Vec<_>>()})
}
fn search(page: u32, total: u64) -> Value {
    let start = u64::from(page - 1) * 20;
    let items = (start..total.min(start + 20))
        .map(|n| {
            json!({"MvID":n+1,"MvName":format!("Video {}",n+1),
        "Duration":123,"AudioID":n+51,"MixSongID":n+81,"Pic":"bare.jpg",
        "Singers":[{"id":35,"name":"Singer"}]})
        })
        .collect::<Vec<_>>();
    json!({"status":1,"error_code":0,"data":{"page":page,"pagesize":20,"from":start,"size":20,"total":total,"lists":items}})
}
fn query(limit: u32, offset: u32) -> SearchQuery {
    let mut q = SearchQuery::tracks("测试", limit, offset);
    q.kind = SearchKind::Mv;
    q
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
async fn video_batch_uses_anonymous_body_only_signature_and_preserves_order_and_duplicates() {
    let mut f = server(vec![
        raw(details((1..=20).rev())).into(),
        raw(details([21])).into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    let old = credential("999", "account-secret");
    store.put(&old.stored("default").unwrap()).unwrap();
    f.provider.credential_store = Some(store.clone());
    let mut ids = (1..=21).map(|i| format!("mv:{i}")).collect::<Vec<_>>();
    ids.extend(["1".into(), "mv:21".into()]);
    let result = f
        .provider
        .videos(&ids, &VideoDetailRequest::new(VideoResourceKind::Mv))
        .await
        .unwrap();
    assert_eq!(result.len(), 23);
    for (detail, id) in result.iter().zip(&ids) {
        assert_eq!(detail.video.resource_ref.id(), id);
    }
    assert_eq!(
        result
            .iter()
            .map(|v| v.video.id.clone())
            .collect::<Vec<_>>(),
        (1..=21)
            .chain([1, 21])
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
    );
    let reqs = f.requests.await.unwrap();
    assert_eq!(reqs.len(), 2);
    for r in reqs {
        assert!(r.starts_with("POST /v1/video?"));
        let q = params(&r);
        assert_eq!(q.len(), 1);
        let bytes = r.split_once("\r\n\r\n").unwrap().1.as_bytes();
        assert_eq!(
            q["signature"],
            crate::signing::android_signature(&BTreeMap::new(), bytes)
        );
        let b: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(b["appid"], 1005);
        assert_eq!(b["clientver"], 20489);
        assert_eq!(b["dfid"], "-");
        assert_eq!(b["token"], "");
        assert_eq!(b["show_resolution"], 1);
        use md5::{Digest, Md5};
        let hash = |s: String| hex::encode(Md5::digest(s.as_bytes()));
        assert_eq!(b["uuid"], hash(format!("-{}", b["mid"].as_str().unwrap())));
        assert_eq!(
            b["key"],
            hash(format!(
                "1005{}20489{}",
                crate::signing::ANDROID_SALT,
                b["clienttime"]
            ))
        );
        assert!(r.to_lowercase().contains("x-router: kmr.service.kugou.com"));
        assert!(
            r.to_lowercase()
                .contains(&format!("clienttime: {}", b["clienttime"]))
        );
        assert!(!r.to_lowercase().contains("cookie:"));
        assert!(!r.contains("account-secret"));
        assert!(!r.to_lowercase().contains("authorization:"));
        assert!(!b.as_object().unwrap().contains_key("userid"));
    }
    assert_eq!(
        store.values.lock().unwrap().get("default").unwrap(),
        &old.stored("default").unwrap()
    );
}

#[tokio::test]
async fn mv_search_crosses_pages_enriches_actual_video_time_and_cover_and_keeps_pagination() {
    let f = server(vec![
        raw(search(1, 28)).into(),
        raw(details(1..=20)).into(),
        raw(search(2, 28)).into(),
        raw(details(21..=28)).into(),
    ])
    .await;
    let p = f.provider.search_catalog(&query(8, 17)).await.unwrap();
    assert_eq!(p.items.len(), 8);
    assert_eq!(p.pagination.total, Some(28));
    assert_eq!(p.pagination.next_offset, Some(25));
    assert!(p.pagination.has_more);
    let SearchItem::Video(v) = &p.items[0] else {
        panic!()
    };
    assert_eq!(v.id, "18");
    assert_eq!(v.duration_ms, Some(123456));
    assert_eq!(v.extensions["search_duration_seconds"], 123);
    assert_eq!(
        v.cover_url.as_deref(),
        Some("https://imge.kugou.com/mvhdpic/400/date/cover.jpg")
    );
    assert_eq!(v.extensions["detail_enriched"], true);
    assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 2);
    let reqs = f.requests.await.unwrap();
    assert_eq!(reqs.len(), 4);
    assert!(reqs[0].starts_with("GET /v1/search/mv?"));
    assert_eq!(params(&reqs[2])["page"], "2");
    assert_eq!(params(&reqs[0])["pagesize"], "20");
    for r in [&reqs[0], &reqs[2]] {
        let mut p = params(r);
        let signature = p.remove("signature").unwrap();
        assert_eq!(
            signature,
            crate::signing::android_signature(
                &p.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
                b""
            )
        );
    }
}

#[tokio::test]
async fn mv_search_empty_and_out_of_range_do_not_trigger_detail_requests_or_invent_totals() {
    let f = server(vec![raw(search(1, 0)).into()]).await;
    let p = f.provider.search_catalog(&query(10, 0)).await.unwrap();
    assert!(p.items.is_empty());
    assert_eq!(p.pagination.total, Some(0));
    assert!(!p.pagination.has_more);
    assert_eq!(f.requests.await.unwrap().len(), 1);
    let f = server(vec![
        raw(json!({"status":1,"error_code":149,"data":{"total":0,"lists":[]}})).into(),
    ])
    .await;
    assert_eq!(
        f.provider
            .search_catalog(&query(10, 500))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn mv_enrichment_rejects_cross_resource_or_changed_video_metadata_without_partial_results() {
    for (field, value) in [
        ("video_id", json!(2)),
        ("audio_id", json!(999)),
        ("album_audio_id", json!(999)),
        ("timelength", json!(100000)),
    ] {
        let mut d = details([1]);
        d["data"][0][field] = value;
        let f = server(vec![raw(search(1, 1)).into(), raw(d).into()]).await;
        assert_eq!(
            f.provider
                .search_catalog(&query(1, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "{field}"
        );
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
    let f = server(vec![
        raw(details(1..=20)).into(),
        raw(json!({"status":1,"error_code":0,"data":[{}]})).into(),
    ])
    .await;
    let e = f
        .provider
        .videos(
            &(1..=21).map(|i| i.to_string()).collect::<Vec<_>>(),
            &VideoDetailRequest::new(VideoResourceKind::Mv),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::ResourceNotFound);
    assert_eq!(f.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn public_video_kinds_share_numeric_identity_but_reject_conflicting_prefixes_accounts_and_batches()
 {
    let f = server(vec![]).await;
    for kind in [VideoResourceKind::Mv, VideoResourceKind::Video] {
        let request = VideoDetailRequest::new(kind);
        for id in [
            "",
            "0",
            "01",
            "+1",
            " 1",
            "hash",
            "chart:1",
            if kind == VideoResourceKind::Mv {
                "video:1"
            } else {
                "mv:1"
            },
        ] {
            assert_eq!(
                f.provider.video(id, &request).await.unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        for ids in [vec![], vec!["1".into(); 101]] {
            assert_eq!(
                f.provider.videos(&ids, &request).await.unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        let mut r = request.clone();
        r.account = Some("named".into());
        assert_eq!(
            f.provider.video("1", &r).await.unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
        let scoped = f
            .provider
            .caller_scope(&credential("999", "private").caller().unwrap())
            .unwrap();
        assert_eq!(
            scoped.video("not-an-id", &request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let mut q = query(1, 0);
    q.video_filters = Some(VideoSearchFilters::default());
    assert_eq!(
        f.provider.search_catalog(&q).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
    let f = server(vec![raw(details([1])).into()]).await;
    let v = f
        .provider
        .video(
            "video:1",
            &VideoDetailRequest::new(VideoResourceKind::Video),
        )
        .await
        .unwrap();
    assert_eq!(v.kind, VideoResourceKind::Video);
    assert_eq!(v.video.resource_ref.to_string(), "kugou:video:1");
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn video_transport_rejects_redirects_challenges_wrong_mime_limits_and_business_errors_once() {
    let good = raw(details([1]));
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: https://untrusted.invalid/\r\nContent-Length: 0\r\n\r\n"
            .into(),
        good.replace("Content-Type: application/json", "Content-Type: text/html"),
        good.replacen("\r\n", "\r\nssa-code: challenge\r\n", 1),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\n\r\n"
            .into(),
        raw(json!({"status":0,"error_code":77,"data":[],"errmsg":"never-export"})),
        raw(json!({"status":1,"error_code":0,"errcode":23,"data":[video(1)]})),
        raw(json!({"status":1,"error_code":0,"data":[{"video_id":1}]})),
    ] {
        let f = server(vec![response.into()]).await;
        let e = f
            .provider
            .video("1", &VideoDetailRequest::new(VideoResourceKind::Mv))
            .await
            .unwrap_err();
        assert!(!format!("{e:?}").contains("never-export"));
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
#[ignore = "uses live official anonymous MV search and video details"]
async fn live_mv_search_and_video_detail_crosscheck_identity_duration_cover_and_missing_resources()
{
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    let mut q = query(5, 18);
    q.query = "周杰伦".into();
    let p = provider.search_catalog(&q).await.unwrap();
    assert_eq!(p.items.len(), 5);
    assert_eq!(p.pagination.next_offset, Some(23));
    let ids = p
        .items
        .iter()
        .map(|i| match i {
            SearchItem::Video(v) => v.id.clone(),
            _ => panic!(),
        })
        .collect::<Vec<_>>();
    let d = provider
        .videos(&ids, &VideoDetailRequest::new(VideoResourceKind::Mv))
        .await
        .unwrap();
    for (item, detail) in p.items.iter().zip(d) {
        let SearchItem::Video(v) = item else { panic!() };
        assert_eq!(v.id, detail.video.id);
        assert_eq!(v.duration_ms, detail.video.duration_ms);
        assert_eq!(v.cover_url, detail.video.cover_url);
        assert!(v.cover_url.is_some());
    }
    let video = provider
        .video(
            "video:17781851",
            &VideoDetailRequest::new(VideoResourceKind::Video),
        )
        .await
        .unwrap();
    assert_eq!(video.video.id, "17781851");
    assert_eq!(video.video.extensions["uploader"]["id"], "2461945934");
    assert_eq!(
        provider
            .video(
                "999999999999",
                &VideoDetailRequest::new(VideoResourceKind::Mv)
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
}
