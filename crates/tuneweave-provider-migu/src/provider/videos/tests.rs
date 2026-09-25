use super::*;
use crate::client::videos::tests::{artist, detail, search};
use crate::provider::catalog::tests::server;
use serde_json::Value;
use tuneweave_core::VideoSearchFilters;
fn response(v: Value) -> String {
    let body = v.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn info() -> String {
    response(json!({"code":"000000","data":crate::client::artists::tests::info("112",None,None)}))
}
fn artist_request(limit: u32, offset: u32) -> ArtistVideoListRequest {
    ArtistVideoListRequest {
        kind: VideoKind::Mv,
        ..ArtistVideoListRequest::new(limit, offset)
    }
}
fn query(limit: u32, offset: u32) -> SearchQuery {
    SearchQuery {
        kind: SearchKind::Mv,
        ..SearchQuery::tracks(" A&B / 中文 ", limit, offset)
    }
}
#[tokio::test]
async fn mv_search_crosses_real_physical_pages_and_encodes_order_without_credentials() {
    for (order, value) in [
        (VideoSearchOrder::Relevance, 0),
        (VideoSearchOrder::Newest, 1),
        (VideoSearchOrder::MostPlayed, 2),
    ] {
        let (p, requests) = server(vec![
            response(search(1, 20, true)),
            response(search(21, 3, false)),
        ])
        .await;
        let mut q = query(6, 18);
        q.video_filters = Some(VideoSearchFilters {
            order,
            ..Default::default()
        });
        let r = p.search_catalog(&q).await.unwrap();
        assert_eq!(r.items.len(), 5);
        assert!(!r.pagination.has_more);
        assert!(r.pagination.total.is_none());
        assert!(matches!(&r.items[0],SearchItem::Video(v) if v.id=="19"));
        for (i, r) in requests.await.unwrap().iter().enumerate() {
            let target = r.lines().next().unwrap().split_whitespace().nth(1).unwrap();
            let u = url::Url::parse(&format!("http://localhost{target}")).unwrap();
            let pairs = u
                .query_pairs()
                .into_owned()
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(u.path(), videos::SEARCH_PATH);
            assert_eq!(pairs["text"], "A&B / 中文");
            assert_eq!(pairs["pageNo"], (i + 1).to_string());
            assert_eq!(pairs["typeOrder"], value.to_string());
            assert!(
                !r.to_lowercase().contains("cookie:") && !r.to_lowercase().contains("pacmtoken:")
            );
        }
    }
}
#[tokio::test]
async fn mv_artist_verifies_profile_and_keeps_mv_pages_separate_from_mixed_video() {
    let (p, requests) = server(vec![
        info(),
        response(artist(1, 10, 1, true)),
        response(artist(11, 3, 2, false)),
    ])
    .await;
    let r = p.artist_videos("112", &artist_request(6, 8)).await.unwrap();
    assert_eq!(
        r.items.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        ["9", "10", "11", "12", "13"]
    );
    assert!(!r.pagination.has_more);
    assert!(r.pagination.total.is_none());
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 3);
    for (i, r) in requests[1..].iter().enumerate() {
        assert!(r.starts_with(&format!(
            "GET {}?singerId=112&pageNo={} ",
            videos::ARTIST_PATH,
            i + 1
        )));
        assert!(!r.contains("pacmtoken:"));
    }
    let (p, requests) = server(vec![info(), response(artist(21, 0, 3, false))]).await;
    assert!(
        p.artist_videos("112", &artist_request(3, 20))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(requests.await.unwrap().len(), 2);
}
#[tokio::test]
async fn mv_detail_and_stats_use_fixed_public_resource_identity_without_playback_requests() {
    let (p, requests) = server(vec![response(detail("7")), response(detail("7"))]).await;
    let r = VideoDetailRequest::new(VideoResourceKind::Mv);
    assert_eq!(
        p.video("7", &r).await.unwrap().video.duration_ms,
        Some(259000)
    );
    assert_eq!(
        p.video_stats("7", &r).await.unwrap().view_count,
        Some(283133)
    );
    for request in requests.await.unwrap() {
        assert!(request.starts_with(
            "GET /MIGUM2.0/v1.0/content/resourceinfo.do?resourceId=7&resourceType=D&needSimple=01 "
        ));
        assert!(!request.contains("cookie:") && !request.contains("pacmtoken:"));
    }
}
#[tokio::test]
async fn mv_partial_pages_duplicates_and_late_transport_failure_never_return_partial_success() {
    for artist_mode in [false, true] {
        for case in 0..4 {
            let mut frames = if artist_mode {
                vec![info(), response(artist(1, 10, 1, true))]
            } else {
                vec![response(search(1, 20, true))]
            };
            let mut last = if artist_mode {
                artist(11, 3, 2, false)
            } else {
                search(21, 3, false)
            };
            match case {
                0 => {
                    last = if artist_mode {
                        artist(1, 3, 2, false)
                    } else {
                        search(1, 3, false)
                    };
                }
                1 => last["code"] = json!("299999"),
                2 => {
                    if artist_mode {
                        last["data"]["header"]["nextPageUrl"] = json!("https://evil.invalid/x")
                    } else {
                        last["data"]["hasNext"] = json!(true)
                    }
                }
                _ => {}
            }
            frames.push(if case == 3 {
                "HTTP/1.1 503 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
            } else {
                response(last)
            });
            let (p, requests) = server(frames).await;
            let result = if artist_mode {
                p.artist_videos("112", &artist_request(5, 8))
                    .await
                    .map(|_| ())
            } else {
                p.search_catalog(&query(5, 18)).await.map(|_| ())
            };
            assert!(result.is_err());
            assert_eq!(
                requests.await.unwrap().len(),
                if artist_mode { 3 } else { 2 }
            );
        }
    }
}
#[tokio::test]
async fn mv_public_preflight_rejects_accounts_wrong_kinds_filters_and_bad_pagination() {
    let (p, requests) = server(vec![]).await;
    for id in ["", "01", "x/7", &"1".repeat(65)] {
        assert!(
            p.video(id, &VideoDetailRequest::new(VideoResourceKind::Mv))
                .await
                .is_err()
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert!(p.search_catalog(&query(limit, offset)).await.is_err());
        assert!(
            p.artist_videos("112", &artist_request(limit, offset))
                .await
                .is_err()
        );
    }
    let mut d = VideoDetailRequest::new(VideoResourceKind::Mv);
    d.account = Some("A".into());
    assert!(p.video("7", &d).await.is_err());
    assert!(
        p.video("7", &VideoDetailRequest::new(VideoResourceKind::Video))
            .await
            .is_err()
    );
    assert!(
        p.artist_videos("112", &ArtistVideoListRequest::new(1, 0))
            .await
            .is_err()
    );
    for case in 0..4 {
        let mut r = artist_request(1, 0);
        match case {
            0 => r.cursor = Some("next".into()),
            1 => r.order = Some("hot".into()),
            2 => r.account = Some("A".into()),
            _ => r.kind = VideoKind::All,
        }
        assert!(p.artist_videos("112", &r).await.is_err());
    }
    for case in 0..5 {
        let mut q = query(1, 0);
        q.video_filters = Some(VideoSearchFilters::default());
        match case {
            0 => q.account = Some("A".into()),
            1 => q.video_filters.as_mut().unwrap().category_id = Some("a".into()),
            2 => q.video_filters.as_mut().unwrap().duration = VideoSearchDuration::UnderTenMinutes,
            3 => q.video_filters.as_mut().unwrap().order = VideoSearchOrder::MostFavorited,
            _ => q.highlight = true,
        }
        assert!(p.search_catalog(&q).await.is_err());
    }
    let c = crate::credential::MiguCredential::verified("111".into(), "fixture".into()).unwrap();
    let caller = p.caller_scope(&c.caller().unwrap()).unwrap();
    assert!(caller.search_catalog(&query(1, 0)).await.is_err());
    assert!(
        caller
            .artist_videos("112", &artist_request(1, 0))
            .await
            .is_err()
    );
    assert!(requests.await.unwrap().is_empty());
}
#[tokio::test]
async fn mv_public_transport_enforces_cumulative_and_individual_budgets_mime_and_business_status() {
    let (p, requests) = server(vec![response(detail("7")), response(detail("7"))]).await;
    let mut budget = detail("7").to_string().len() as u64 + 10;
    p.client.mv_detail("7", &mut budget).await.unwrap();
    assert_eq!(budget, 10);
    assert!(p.client.mv_detail("7", &mut budget).await.is_err());
    assert_eq!(requests.await.unwrap().len(), 2);
    for response in ["HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".into(),response(json!({"code":"299999","resource":[]})),response(json!({"resource":[]}))] {
        let (p,requests)=server(vec![response]).await;assert!(p.video("7",&VideoDetailRequest::new(VideoResourceKind::Mv)).await.is_err());assert_eq!(requests.await.unwrap().len(),1);
    }
}
#[tokio::test]
#[ignore = "official anonymous Migu MV metadata only; no account or media bytes"]
async fn live_migu_mv_catalogues_bind_search_detail_stats_and_artist_pagination() {
    let p = MiguProvider::new(MiguConfig::default()).unwrap();
    let mut q = query(3, 18);
    q.query = "周杰伦".into();
    let found = p.search_catalog(&q).await.unwrap();
    assert_eq!(found.items.len(), 3);
    let SearchItem::Video(v) = &found.items[0] else {
        panic!("MV result")
    };
    let d = p
        .video(&v.id, &VideoDetailRequest::new(VideoResourceKind::Mv))
        .await
        .unwrap();
    assert_eq!(d.video.id, v.id);
    assert_eq!(d.video.duration_ms, v.duration_ms);
    assert!(d.resolutions.is_empty());
    let stats = p
        .video_stats(&v.id, &VideoDetailRequest::new(VideoResourceKind::Mv))
        .await
        .unwrap();
    assert_eq!(stats.video_ref, d.video.resource_ref);
    assert!(stats.liked.is_none());
    let list = p.artist_videos("112", &artist_request(5, 8)).await.unwrap();
    assert_eq!(list.items.len(), 5);
    assert!(
        list.items
            .iter()
            .all(|v| v.extensions["resource_type"] == "D")
    );
    assert_eq!(list.pagination.extensions["upstream_pages_fetched"], 2);
}
