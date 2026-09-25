use super::*;
use crate::{ChartTrackListRequest, ErrorCode, MusicProvider, Page, PageMeta, PageRequest, Track};
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn chart_period_dates_validate_gregorian_calendar_and_round_trip_without_weekday_adjustment() {
    for date in [
        "0001-01-01",
        "2000-02-29",
        "2024-02-29",
        "2026-09-14",
        "9999-12-31",
    ] {
        for period in [
            ChartPeriod::Day { date: date.into() },
            ChartPeriod::Week { date: date.into() },
        ] {
            period.validate().unwrap();
            let encoded = serde_json::to_value(&period).unwrap();
            assert_eq!(encoded["date"], date);
            assert_eq!(
                serde_json::from_value::<ChartPeriod>(encoded).unwrap(),
                period
            );
            assert_eq!(period.date(), Some(date));
        }
    }
    for date in [
        "",
        "0000-01-01",
        "1900-02-29",
        "2100-02-29",
        "2026-02-29",
        "2026-04-31",
        "2026-00-01",
        "2026-13-01",
        "2026-01-00",
        "2026-1-01",
        "20260101",
        "2026-01-01Z",
        "２０２６-01-01",
        "2026-01-0\n",
    ] {
        assert!(
            ChartPeriod::Day { date: date.into() }.validate().is_err(),
            "{date}"
        );
    }
}

#[test]
fn chart_period_strict_wire_fields_and_legacy_requests_keep_current_default() {
    for body in [
        r#"{"kind":"current","date":"2026-01-01"}"#,
        r#"{"kind":"current","extra":true}"#,
        r#"{"kind":"day"}"#,
        r#"{"kind":"day","date":null}"#,
        r#"{"kind":"day","date":"2026-02-29"}"#,
        r#"{"kind":"day","date":"2026-01-01","extra":true}"#,
        r#"{"kind":"day","date":"2026-01-01","date":"2026-02-01"}"#,
        r#"{"kind":"week","kind":"day","date":"2026-01-01"}"#,
        r#"{"kind":"month","date":"2026-01-01"}"#,
        r#"null"#,
        r#"{"kind":"id","id":"7","date":"2026-01-01"}"#,
        r#"{"kind":"id"}"#,
        r#"{"kind":"id","id":"7","id":"8"}"#,
    ] {
        assert!(serde_json::from_str::<ChartPeriod>(body).is_err(), "{body}");
    }
    let r: ChartTrackListRequest =
        serde_json::from_value(json!({"limit":10,"offset":20,"include_tags":true,"account":null}))
            .unwrap();
    assert_eq!(r, ChartTrackListRequest::new(10, 20));
    assert_eq!(r.period, ChartPeriod::Current);
    assert_eq!(
        serde_json::to_value(Capability::ChartHistoricalTracks).unwrap(),
        "chart_historical_tracks"
    );
}

struct DefaultCharts {
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl MusicProvider for DefaultCharts {
    fn platform(&self) -> Platform {
        Platform::Netease
    }
    fn name(&self) -> &'static str {
        "default chart fallback test"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::ChartTracks])
    }
    async fn playlist_tracks(&self, _: &str, r: &PageRequest) -> Result<Page<Track>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Page {
            items: vec![],
            pagination: PageMeta {
                limit: r.limit,
                offset: r.offset,
                total: Some(0),
                has_more: false,
                next_offset: None,
                extensions: Default::default(),
            },
        })
    }
}
#[tokio::test]
async fn chart_period_default_provider_never_falls_through_history_to_a_playlist() {
    let p = DefaultCharts {
        calls: AtomicUsize::new(0),
    };
    let mut r = ChartTrackListRequest::new(1, 0);
    p.chart_tracks("chart", &r).await.unwrap();
    for period in [
        ChartPeriod::Id { id: "7".into() },
        ChartPeriod::Day {
            date: "2026-01-01".into(),
        },
        ChartPeriod::Week {
            date: "2026-09-14".into(),
        },
    ] {
        r.period = period;
        let e = p.chart_tracks("chart", &r).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::CapabilityNotSupported);
        assert_eq!(e.platform, Some(Platform::Netease));
    }
    r.period = ChartPeriod::Day {
        date: "2026-02-29".into(),
    };
    assert_eq!(
        p.chart_tracks("chart", &r).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn chart_period_opaque_ids_are_bounded_strict_and_do_not_become_dates() {
    let p = ChartPeriod::Id {
        id: "provider:2026_7".into(),
    };
    p.validate().unwrap();
    assert_eq!(p.kind(), "id");
    assert_eq!(p.date(), None);
    assert_eq!(serde_json::from_value::<ChartPeriod>(json!(p)).unwrap(), p);
    for id in ["".to_owned(), "a b".into(), "7\n".into(), "x".repeat(129)] {
        assert!(ChartPeriod::Id { id }.validate().is_err());
    }
}
