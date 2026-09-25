use super::*;
use tuneweave_core::ChartPeriod;

fn history(total: u32) -> Value {
    let mut d = detail(total);
    d["data"]["periodColumnId"] = json!("2001");
    d["data"]["rankTypeList"] = json!(["0", "1", "2"]);
    d["data"]["dayRankUpdateTime"] = json!("20200309");
    d["data"]["weekRankUpdateTime"] = json!("20200313");
    d
}
fn req(period: ChartPeriod, limit: u32, offset: u32) -> ChartTrackListRequest {
    let mut r = ChartTrackListRequest::new(limit, offset);
    r.period = period;
    r
}

#[tokio::test]
async fn chart_history_exact_queries_complete_windows_and_no_default_credentials() {
    let day = ChartPeriod::Day {
        date: "2026-01-01".into(),
    };
    let week = ChartPeriod::Week {
        date: "2026-09-14".into(),
    };
    let (mut p, requests) = server(vec![
        response(history(49)),
        response(history(49)),
        response(history(49)),
    ])
    .await;
    p.credential_store = Some(Arc::new(NoStore));
    for (period, offset, count) in [(day.clone(), 47, 2), (week, 48, 1), (day, 49, 0)] {
        let r = p
            .chart_tracks("chart:20", &req(period.clone(), 5, offset))
            .await
            .unwrap();
        assert_eq!(r.pagination.total, Some(49));
        assert_eq!(r.items.len(), count);
        assert!(!r.pagination.has_more);
        assert!(r.pagination.next_offset.is_none());
        assert_eq!(r.pagination.extensions["requested_period"], json!(period));
        assert_eq!(r.pagination.extensions["period_column_id"], "2001");
        assert_eq!(r.pagination.extensions["upstream_pages_fetched"], 1);
    }
    for (r, query) in requests.await.unwrap().iter().zip([
        "rankType=1&period=20260101",
        "rankType=2&period=20260914",
        "rankType=1&period=20260101",
    ]) {
        assert!(r.starts_with(&format!("GET {}?rankId=20&{query} ", charts::TRACKS_PATH)));
        for secret in ["cookie:", "pacmtoken:", "global-token:", "authorization:"] {
            assert!(!r.to_lowercase().contains(secret));
        }
    }
}

#[tokio::test]
async fn chart_history_unavailable_current_fallback_and_bad_tail_never_return_partial_or_retry() {
    let p = ChartPeriod::Day {
        date: "2026-01-01".into(),
    };
    let mut bad = history(49);
    bad["data"]["contents"][48]["songId"] = json!("777");
    for (body, code) in [
        (json!({"code":"200000"}), ErrorCode::ResourceNotFound),
        (detail(1), ErrorCode::UpstreamError),
        (bad, ErrorCode::UpstreamError),
    ] {
        let (provider, requests) = server(vec![response(body)]).await;
        assert_eq!(
            provider
                .chart_tracks("20", &req(p.clone(), 1, 0))
                .await
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn chart_history_invalid_dates_accounts_and_callers_fail_before_io() {
    let (mut p, requests) = server(vec![]).await;
    p.credential_store = Some(Arc::new(NoStore));
    for date in ["2026-02-29", "2026-01-01&rankType=0", "2026-09-14 ", ""] {
        assert_eq!(
            p.chart_tracks("20", &req(ChartPeriod::Week { date: date.into() }, 1, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut r = req(
        ChartPeriod::Day {
            date: "2026-01-01".into(),
        },
        1,
        0,
    );
    r.account = Some("default".into());
    assert!(p.chart_tracks("20", &r).await.is_err());
    r.account = None;
    let c = crate::credential::MiguCredential::verified("111".into(), "fixture".into()).unwrap();
    let caller = p.caller_scope(&c.caller().unwrap()).unwrap();
    assert!(caller.chart_tracks("20", &r).await.is_err());
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "Official anonymous historical chart metadata only; no accounts or audio/video"]
async fn live_migu_chart_history_day_week_and_unavailable_periods() {
    let p = MiguProvider::new(MiguConfig::default()).unwrap();
    let mut ids = BTreeSet::new();
    for period in [
        ChartPeriod::Day {
            date: "2026-01-01".into(),
        },
        ChartPeriod::Week {
            date: "2026-09-11".into(),
        },
    ] {
        let r = p
            .chart_tracks("27553319", &req(period.clone(), 3, 18))
            .await
            .unwrap();
        assert_eq!(r.items.len(), 3);
        assert_eq!(r.items[0].extensions["chart_rank"], 19);
        let id = r.pagination.extensions["period_column_id"]
            .as_str()
            .unwrap();
        assert_ne!(id, "27553319");
        assert!(ids.insert(id.to_owned()));
        assert_eq!(r.pagination.extensions["requested_period"], json!(period));
        let t = &r.items[0];
        let d = p.track(&t.id, None).await.unwrap();
        assert_eq!(d.resource_ref, t.resource_ref);
        assert_eq!(d.extensions["song_id"], t.extensions["song_id"]);
        assert_eq!(d.extensions["copyright_id"], t.extensions["copyright_id"]);
        let total = r.pagination.total.unwrap() as u32;
        let last = p
            .chart_tracks("27553319", &req(period.clone(), 5, total - 1))
            .await
            .unwrap();
        assert_eq!(last.items.len(), 1);
        assert!(!last.pagination.has_more);
        let beyond = p
            .chart_tracks("27553319", &req(period, 5, total))
            .await
            .unwrap();
        assert!(beyond.items.is_empty());
        assert_eq!(beyond.pagination.total, Some(total.into()));
    }
    for date in ["2049-09-17", "2019-01-01"] {
        assert_eq!(
            p.chart_tracks(
                "27553319",
                &req(ChartPeriod::Day { date: date.into() }, 1, 0)
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::ResourceNotFound
        );
    }
}
