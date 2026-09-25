use super::*;
use crate::client::charts::periods::tests::{metadata, periods};
use tuneweave_core::StoredAccountCredential;

fn request(id: &str, limit: u32, offset: u32) -> ChartTrackListRequest {
    let mut r = ChartTrackListRequest::new(limit, offset);
    r.period = ChartPeriod::Id { id: id.into() };
    r
}
fn historical_tracks(page: u32, total: u64) -> Value {
    let mut v = tracks(page, total);
    for row in v["data"]["songlist"].as_array_mut().unwrap() {
        row["rank_cid"] = json!(6);
        row["business"]["rank_id"] = json!("6");
        let n = row["business"]["sort"].as_u64().unwrap();
        row["business"]["sort"] = json!(n + 1);
    }
    v
}
struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public periods read stored credentials")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public periods wrote credentials")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public periods removed credentials")
    }
}

#[tokio::test]
async fn chart_history_period_discovery_returns_complete_windows_with_exact_public_signature() {
    for (offset, count, more) in [(0, 2, true), (2, 1, false), (3, 0, false)] {
        let mut f = server(vec![html(metadata(200, 7)).into(), html(periods()).into()]).await;
        f.provider.credential_store = Some(Arc::new(NoStore));
        let p = f
            .provider
            .chart_periods("chart:42", &PageRequest::new(2, offset))
            .await
            .unwrap();
        assert_eq!(p.items.len(), count);
        assert_eq!(p.pagination.total, Some(3));
        assert_eq!(p.pagination.has_more, more);
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        let (url, q, body) = signed(&requests[1]);
        assert_eq!(url.path(), "/ocean/v6/rank/vol");
        assert!(body.is_empty());
        for (key, value) in [
            ("rankid", "42"),
            ("ranktype", "2"),
            ("rank_cid", "7"),
            ("zone", "tx6_gz_kmr"),
            ("plat", "2"),
        ] {
            assert_eq!(q[key], value);
        }
        assert!(!q.contains_key("type"));
    }
}

#[tokio::test]
async fn chart_history_selects_listed_period_pins_all_pages_and_keeps_display_rank_gaps() {
    for offset in [98, 199, 200] {
        let mut f = server(vec![
            html(metadata(250, 7)).into(),
            html(periods()).into(),
            html(metadata(200, 6)).into(),
            raw(historical_tracks(1, 200)).into(),
            raw(historical_tracks(2, 200)).into(),
        ])
        .await;
        f.provider.credential_store = Some(Arc::new(NoStore));
        let p = f
            .provider
            .chart_tracks("42", &request("6", 5, offset))
            .await
            .unwrap();
        assert_eq!(p.pagination.total, Some(200));
        assert_eq!(
            p.items.len(),
            (200usize.saturating_sub(offset as usize)).min(5)
        );
        assert_eq!(p.pagination.extensions["rank_cid"], "6");
        assert_eq!(
            p.pagination.extensions["selected_period"]["period"],
            json!({"kind":"id","id":"6"})
        );
        if let Some(t) = p.items.first() {
            assert_eq!(t.extensions["chart_rank"], offset + 2);
            assert_eq!(t.extensions["original_index"], offset + 1);
        }
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 5);
        for (i, r) in requests.iter().enumerate() {
            let (url, q, body) = signed(r);
            if i == 2 {
                assert_eq!(q["rank_cid"], "6");
                assert_eq!(q["rankid"], "42");
            }
            if i >= 3 {
                assert_eq!(url.path(), "/openapi/kmr/v2/rank/audio");
                let b: Value = serde_json::from_str(body).unwrap();
                assert_eq!(b["rank_id"], 42);
                assert_eq!(b["rank_cid"], 6);
                assert_eq!(b["page"], i - 2);
            }
        }
    }
}

#[tokio::test]
async fn chart_history_unlisted_period_current_fallback_and_off_window_errors_never_return_partial()
{
    for case in 0..6 {
        let mut frames = vec![html(metadata(200, 7)).into(), html(periods()).into()];
        if case > 0 {
            frames.push(html(metadata(200, if case == 1 { 7 } else { 6 })).into());
        }
        if case > 1 {
            frames.push(raw(historical_tracks(1, 200)).into());
            let mut v = historical_tracks(2, 200);
            match case {
                2 => v["data"]["songlist"][99]["business"]["rank_id"] = json!("7"),
                3 => v["data"]["songlist"][99]["business"]["parent_id"] = json!("43"),
                4 => v["data"]["songlist"][99]["business"]["original_index"] = json!(199),
                _ => v["data"]["songlist"][99]["album_audio_id"] = json!(1000),
            }
            frames.push(raw(v).into());
        }
        let expected = frames.len();
        let f = server(frames).await;
        let e = f
            .provider
            .chart_tracks("42", &request(if case == 0 { "123456" } else { "6" }, 1, 0))
            .await
            .unwrap_err();
        assert_eq!(
            e.code,
            if case == 0 {
                ErrorCode::ResourceNotFound
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert_eq!(f.requests.await.unwrap().len(), expected);
    }
}

#[tokio::test]
async fn chart_history_rejects_accounts_callers_dates_and_noncanonical_ids_before_io() {
    let mut f = server(vec![]).await;
    f.provider.credential_store = Some(Arc::new(NoStore));
    for id in ["0", "06", "+6", "chart:6", "18446744073709551616", ""] {
        assert_eq!(
            f.provider
                .chart_tracks("42", &request(id, 1, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert!(
            f.provider
                .chart_periods("42", &PageRequest::new(limit, offset))
                .await
                .is_err()
        );
    }
    let mut r = request("6", 1, 0);
    r.account = Some("default".into());
    assert!(f.provider.chart_tracks("42", &r).await.is_err());
    let mut page = PageRequest::new(1, 0);
    page.account = Some("default".into());
    assert!(f.provider.chart_periods("42", &page).await.is_err());
    let caller = f
        .provider
        .caller_scope(&credential("999", "fixture").caller().unwrap())
        .unwrap();
    assert!(
        caller
            .chart_tracks("42", &request("6", 1, 0))
            .await
            .is_err()
    );
    assert!(
        caller
            .chart_periods("42", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    r.account = None;
    r.period = ChartPeriod::Day {
        date: "2026-09-16".into(),
    };
    assert_eq!(
        f.provider.chart_tracks("42", &r).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn chart_history_period_transport_keeps_bounded_body_mime_status_and_no_redirect_rules() {
    let good = html(periods());
    for reply in [
        good.replace("text/html; charset=utf-8", "image/png"),
        good.replace("Content-Type:", "SSA-CODE: hidden-proof\r\nContent-Type:"),
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\n\r\n"
            .into(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 1048577\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n".to_owned()
            + &"x".repeat(1048577),
        html(json!({"status":0,"errcode":20006,"data":{},"error":"hidden-proof"})),
    ] {
        let f = server(vec![html(metadata(200, 7)).into(), reply.into()]).await;
        let e = f
            .provider
            .chart_periods("42", &PageRequest::new(1, 0))
            .await
            .unwrap_err();
        assert!(!format!("{e:?}").contains("hidden-proof"));
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
#[ignore = "Official anonymous historical period catalogue and metadata only; no accounts or media"]
async fn live_kugou_chart_history_period_discovery_complete_tracks_and_identity() {
    let p = KugouProvider::new(crate::KugouConfig::default()).unwrap();
    for chart in ["8888", "90379"] {
        let periods = p
            .chart_periods(chart, &PageRequest::new(3, 0))
            .await
            .unwrap();
        assert!(periods.pagination.total.unwrap() > 3);
        let selected = periods
            .items
            .iter()
            .find(|p| p.is_current == Some(false))
            .unwrap()
            .period
            .clone();
        let mut r = ChartTrackListRequest::new(4, 98);
        r.period = selected;
        let tracks = p.chart_tracks(chart, &r).await.unwrap();
        assert_eq!(tracks.items.len(), 4);
        let t = &tracks.items[0];
        let detail = p.track(&t.id, None).await.unwrap();
        assert_eq!(detail.resource_ref, t.resource_ref);
        let total = tracks.pagination.total.unwrap() as u32;
        r.offset = total - 1;
        assert_eq!(p.chart_tracks(chart, &r).await.unwrap().items.len(), 1);
        r.offset = total;
        assert!(p.chart_tracks(chart, &r).await.unwrap().items.is_empty());
        r.period = ChartPeriod::Id {
            id: "18446744073709551615".into(),
        };
        assert_eq!(
            p.chart_tracks(chart, &r).await.unwrap_err().code,
            ErrorCode::ResourceNotFound
        );
    }
}

#[tokio::test]
async fn chart_history_total_deadline_and_cancellation_cover_each_header_and_body_boundary() {
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::oneshot,
    };
    for catalogue in [true, false] {
        for stage in 0..if catalogue { 2 } else { 5 } {
            for body in [false, true] {
                for cancel in [false, true] {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let origin =
                        url::Url::parse(&format!("http://{}", listener.local_addr().unwrap()))
                            .unwrap();
                    let frames = [
                        html(metadata(200, 7)),
                        html(periods()),
                        html(metadata(200, 6)),
                        raw(historical_tracks(1, 200)),
                        raw(historical_tracks(2, 200)),
                    ];
                    let (at_tx, at_rx) = oneshot::channel();
                    let (release_tx, release_rx) = oneshot::channel();
                    let transport = tokio::spawn(async move {
                        for frame in frames.iter().take(stage) {
                            let (mut socket, _) = listener.accept().await.unwrap();
                            read_request(&mut socket).await;
                            socket.write_all(frame.as_bytes()).await.unwrap();
                        }
                        let (mut socket, _) = listener.accept().await.unwrap();
                        read_request(&mut socket).await;
                        if body {
                            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\n\r\n{").await.unwrap();
                        }
                        let _ = at_tx.send(());
                        let _ = release_rx.await;
                    });
                    let mut p = KugouProvider::new(crate::KugouConfig::default()).unwrap();
                    p.client.login_test_origin = Some(origin);
                    p.client.http = reqwest::Client::builder()
                        .no_proxy()
                        .timeout(Duration::from_secs(600))
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .unwrap();
                    let work = tokio::spawn(async move {
                        if catalogue {
                            p.chart_periods("42", &PageRequest::new(1, 0))
                                .await
                                .map(|_| ())
                        } else {
                            p.chart_tracks("42", &request("6", 1, 0)).await.map(|_| ())
                        }
                    });
                    tokio::time::timeout(Duration::from_secs(5), at_rx)
                        .await
                        .unwrap()
                        .unwrap();
                    if cancel {
                        work.abort();
                        assert!(work.await.unwrap_err().is_cancelled());
                    } else {
                        tokio::time::pause();
                        tokio::time::advance(Duration::from_secs(46)).await;
                        let e = work.await.unwrap().unwrap_err();
                        tokio::time::resume();
                        assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                    }
                    let _ = release_tx.send(());
                    transport.await.unwrap();
                }
            }
        }
    }
    async fn read_request(socket: &mut tokio::net::TcpStream) {
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let size = header
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + size {
                    break;
                }
            }
            assert!(bytes.len() < 262144);
        }
    }
}
