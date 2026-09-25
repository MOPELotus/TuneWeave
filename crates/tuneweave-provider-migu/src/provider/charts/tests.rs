use super::*;
use crate::client::charts::{
    self,
    tests::{catalogue, detail, long_timeout_client},
};
use crate::provider::catalog::tests::server;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::{ChartCatalogView, ErrorCode, StoredAccountCredential};
use url::Url;

mod history;

fn response(v: Value) -> String {
    let body = v.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn cat() -> ChartCatalogRequest {
    ChartCatalogRequest::new(ChartCatalogView::Summary)
}
struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public chart read touched credentials")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public chart read wrote credentials")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public chart read removed credentials")
    }
}

#[tokio::test]
async fn charts_public_catalogue_and_full_read_windows_use_exact_anonymous_requests() {
    let (mut p, requests) = server(vec![
        response(catalogue()),
        response(detail(23)),
        response(detail(23)),
        response(detail(23)),
        response(detail(0)),
    ])
    .await;
    p.credential_store = Some(Arc::new(NoStore));
    let c = p.chart_catalog(&cat()).await.unwrap();
    assert_eq!(c.groups[1].charts[0].name, "国风热歌榜");
    for (offset, limit, expected) in [(18, 10, 5), (22, 10, 1), (200, 100, 0), (0, 5, 0)] {
        let r = p
            .chart_tracks("chart:20", &ChartTrackListRequest::new(limit, offset))
            .await
            .unwrap();
        assert_eq!(r.items.len(), expected);
        assert_eq!(r.pagination.total, Some(if offset == 0 { 0 } else { 23 }));
        assert!(!r.pagination.has_more && r.pagination.next_offset.is_none());
        if expected > 0 {
            assert_eq!(r.items[0].extensions["chart_rank"], offset + 1);
        }
        assert_eq!(r.pagination.extensions["complete_read"], true);
        assert_eq!(r.pagination.extensions["upstream_pages_fetched"], 1);
    }
    for (i, r) in requests.await.unwrap().iter().enumerate() {
        assert!(r.starts_with(&format!(
            "GET {} ",
            if i == 0 {
                charts::INDEX_PATH.into()
            } else {
                format!("{}?rankId=20&rankType=&period=", charts::TRACKS_PATH)
            }
        )));
        let lower = r.to_lowercase();
        for forbidden in ["cookie:", "pacmtoken:", "authorization:", "global-token:"] {
            assert!(!lower.contains(forbidden));
        }
        assert!(lower.contains("origin: https://music.migu.cn\r\n"));
    }
    assert!(p.take_response_credential().unwrap().is_none());
}

#[tokio::test]
async fn charts_preserve_more_pages_and_reject_bad_tail_even_when_window_does_not_include_it() {
    let mut bad = detail(23);
    bad["data"]["contents"][22]["songId"] = json!("9999");
    let (p, requests) = server(vec![response(detail(23)), response(bad)]).await;
    let r = p
        .chart_tracks("20", &ChartTrackListRequest::new(2, 18))
        .await
        .unwrap();
    assert_eq!(r.pagination.total, Some(23));
    assert!(r.pagination.has_more);
    assert_eq!(r.pagination.next_offset, Some(20));
    assert!(
        p.chart_tracks("20", &ChartTrackListRequest::new(1, 0))
            .await
            .is_err()
    );
    assert_eq!(requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn charts_repeated_song_positions_survive_catalogue_and_track_window() {
    let mut c = catalogue();
    let previews = c["data"]["contents"][1]["contents"][0]["contents"]
        .as_array_mut()
        .unwrap();
    previews[2] = previews[0].clone();
    let mut d = detail(4);
    let rows = d["data"]["contents"].as_array_mut().unwrap();
    rows[2] = rows[0].clone();
    rows[3] = rows[0].clone();
    rows[2]["txt5"] = json!("-2");
    rows[3]["txt5"] = json!("3");
    let (p, requests) = server(vec![response(c), response(d)]).await;
    let c = p.chart_catalog(&cat()).await.unwrap();
    let previews = &c.groups[1].charts[0].previews;
    assert_eq!(previews[0].track_ref, previews[2].track_ref);
    assert_eq!(previews[2].rank, Some(3));
    let r = p
        .chart_tracks("20", &ChartTrackListRequest::new(3, 1))
        .await
        .unwrap();
    assert_eq!(r.pagination.total, Some(4));
    assert_eq!(r.pagination.extensions["upstream_unique_track_count"], 2);
    assert_eq!(r.pagination.extensions["duplicates_preserved"], true);
    assert_eq!(
        r.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["600002", "600001", "600001"]
    );
    assert_eq!(r.items[1].extensions["chart_rank"], 3);
    assert_eq!(r.items[2].extensions["chart_rank"], 4);
    assert_eq!(r.items[1].extensions["chart_rank_change"], -2);
    assert_eq!(r.items[2].extensions["chart_rank_change"], 3);
    assert_eq!(requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn charts_preflight_rejects_explicit_accounts_caller_credentials_and_invalid_windows() {
    let (mut p, requests) = server(vec![]).await;
    p.credential_store = Some(Arc::new(NoStore));
    for id in [
        "",
        "0",
        "01",
        "chart:chart:20",
        "20?period=x",
        "20/1",
        &"2".repeat(65),
    ] {
        assert_eq!(
            p.chart_tracks(id, &ChartTrackListRequest::new(1, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert!(
            p.chart_tracks("20", &ChartTrackListRequest::new(limit, offset))
                .await
                .is_err()
        );
    }
    let mut c = cat();
    c.account = Some("default".into());
    assert!(p.chart_catalog(&c).await.is_err());
    let mut r = ChartTrackListRequest::new(1, 0);
    r.account = Some("A".into());
    assert!(p.chart_tracks("20", &r).await.is_err());
    let credential =
        crate::credential::MiguCredential::verified("111".into(), "fixture".into()).unwrap();
    let caller = p.caller_scope(&credential.caller().unwrap()).unwrap();
    assert!(caller.chart_catalog(&cat()).await.is_err());
    assert!(
        caller
            .chart_tracks("20", &ChartTrackListRequest::new(1, 0))
            .await
            .is_err()
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn charts_transport_rejects_bad_mime_business_status_redirects_and_declared_or_streamed_oversize()
 {
    let huge = "x".repeat(2 * 1024 * 1024 + 1);
    for body in [
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(),
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".into(),
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{huge}"),
        "HTTP/1.1 302 Found\r\nLocation: https://elsewhere.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        response(json!({"code":"999999","info":"untrusted"})),
        response(json!({"code":"000000","data":null})),
    ] {
        let (p,requests)=server(vec![body]).await;
        let e=p.chart_tracks("20",&ChartTrackListRequest::new(1,0)).await.unwrap_err();
        assert!(!e.message.contains("untrusted"));assert_eq!(requests.await.unwrap().len(),1);
    }
    let (p, requests) = server(vec![response(json!({"code":"200000"}))]).await;
    assert_eq!(
        p.chart_tracks("20", &ChartTrackListRequest::new(1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn charts_total_deadline_and_cancellation_cover_headers_and_incomplete_body() {
    for scenario in 0..4 {
        for body_boundary in [false, true] {
            for cancel in [false, true] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let origin =
                    Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
                let p = MiguProvider::from_client(long_timeout_client(origin));
                let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
                let (release_tx, release_rx) = tokio::sync::oneshot::channel();
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut raw = Vec::new();
                    loop {
                        let mut buf = [0; 1024];
                        let n = socket.read(&mut buf).await.unwrap();
                        assert!(n > 0);
                        raw.extend_from_slice(&buf[..n]);
                        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    if body_boundary {
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();
                    }
                    seen_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                });
                let provider = p.clone();
                let task = tokio::spawn(async move {
                    if scenario == 0 {
                        provider.chart_catalog(&cat()).await.map(|_| ())
                    } else {
                        let mut request = ChartTrackListRequest::new(1, 0);
                        request.period = match scenario {
                            2 => tuneweave_core::ChartPeriod::Day {
                                date: "2026-01-01".into(),
                            },
                            3 => tuneweave_core::ChartPeriod::Week {
                                date: "2026-09-11".into(),
                            },
                            _ => tuneweave_core::ChartPeriod::Current,
                        };
                        provider.chart_tracks("20", &request).await.map(|_| ())
                    }
                });
                tokio::time::timeout(std::time::Duration::from_secs(5), seen_rx)
                    .await
                    .unwrap()
                    .unwrap();
                if cancel {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    tokio::time::pause();
                    let r = task.await;
                    tokio::time::resume();
                    let e = r.unwrap().unwrap_err();
                    assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                    assert!(e.message.contains("total time budget"));
                }
                release_tx.send(()).unwrap();
                server.await.unwrap();
                assert!(p.take_response_credential().unwrap().is_none());
            }
        }
    }
}

#[tokio::test]
#[ignore = "Official anonymous Migu chart metadata only; no account or media bytes"]
async fn live_migu_charts_all_groups_windows_and_track_detail_identity() {
    let p = MiguProvider::new(MiguConfig::default()).unwrap();
    let catalog = p.chart_catalog(&cat()).await.unwrap();
    assert!(!catalog.groups.is_empty());
    let charts = catalog
        .groups
        .iter()
        .flat_map(|g| &g.charts)
        .collect::<Vec<_>>();
    assert!(charts.len() <= 32);
    let mut rejected_incomplete = 0;
    for chart in &charts {
        let id = chart.resource_ref.as_ref().unwrap().id();
        let result = p.chart_tracks(id, &ChartTrackListRequest::new(4, 18)).await;
        // The current official Douyin chart was independently observed to return 99
        // entries while declaring 100 and no next page. This is a tested rejection,
        // not a successful complete-chart acceptance. A corrected response may pass.
        let r = match result {
            Ok(r) => r,
            Err(e)
                if id == "chart:83049014"
                    && e.code == ErrorCode::UpstreamError
                    && e.details["reason"] == "chart_count_mismatch"
                    && e.details["declared_track_count"] == 100
                    && e.details["received_track_count"] == 99 =>
            {
                eprintln!(
                    "{id}: rejected incomplete upstream response (100 declared / 99 received)"
                );
                rejected_incomplete += 1;
                continue;
            }
            Err(e) => panic!("chart {id}: {e:?}"),
        };
        assert_eq!(r.items.len(), 4);
        assert_eq!(r.items[0].extensions["chart_rank"], 19);
        assert!(r.items.iter().all(|t| t.playable.is_none()));
    }
    eprintln!(
        "{} chart responses accepted; {rejected_incomplete} incomplete chart responses rejected",
        charts.len() - rejected_incomplete
    );
    let id = charts[0].resource_ref.as_ref().unwrap().id();
    let r = p
        .chart_tracks(id, &ChartTrackListRequest::new(1, 0))
        .await
        .unwrap();
    let t = &r.items[0];
    let detail = p.track(&t.id, None).await.unwrap();
    assert_eq!(detail.resource_ref, t.resource_ref);
    assert_eq!(detail.extensions["song_id"], t.extensions["song_id"]);
    assert_eq!(
        detail.extensions["copyright_id"],
        t.extensions["copyright_id"]
    );
    let total = r.pagination.total.unwrap() as u32;
    assert!(total > 0);
    let last = p
        .chart_tracks(id, &ChartTrackListRequest::new(20, total - 1))
        .await
        .unwrap();
    assert_eq!(last.items.len(), 1);
    assert!(!last.pagination.has_more);
    let beyond = p
        .chart_tracks(id, &ChartTrackListRequest::new(20, total))
        .await
        .unwrap();
    assert!(beyond.items.is_empty());
    assert_eq!(beyond.pagination.total, Some(total.into()));
    assert_eq!(
        p.chart_tracks("999999999999999", &ChartTrackListRequest::new(1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
}
