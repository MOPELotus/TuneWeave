use super::*;
use crate::client::charts::tests::detail;
use serde_json::Value;

pub(crate) fn historical_detail(total: u32) -> Value {
    let mut v = detail(total);
    v["data"]["periodColumnId"] = json!("2001");
    v["data"]["rankTypeList"] = json!(["0", "1", "2"]);
    v["data"]["dayRankUpdateTime"] = json!("20200309");
    v["data"]["weekRankUpdateTime"] = json!("20200313");
    v
}

#[test]
fn chart_history_maps_requested_calendar_and_returned_period_without_inventing_echo_dates() {
    for requested in [
        ChartPeriod::Day {
            date: "2026-01-01".into(),
        },
        ChartPeriod::Week {
            date: "2026-09-11".into(),
        },
    ] {
        let r = parse_period_tracks(
            &serde_json::to_vec(&historical_detail(49)).unwrap(),
            "20",
            false,
            &requested,
        )
        .unwrap();
        assert_eq!(r.items.len(), 49);
        assert_eq!(r.extensions["requested_period"], json!(requested));
        assert_eq!(r.extensions["period_scope"], requested.kind());
        assert_eq!(r.extensions["period_column_id"], "2001");
        assert_eq!(r.extensions["update_label"], "2026-09-17");
        assert_eq!(
            r.extensions["period_binding_scope"],
            "request_and_returned_column"
        );
        assert_eq!(r.extensions["period_start_dates"]["week"], "2020-03-13");
        assert_eq!(
            r.extensions["available_period_kinds"],
            json!(["current", "day", "week"])
        );
        assert_eq!(r.items[48].extensions["chart_period_id"], "2001");
        assert_eq!(
            r.items[48].extensions["chart_requested_period"],
            json!(requested)
        );
        assert_eq!(r.items[48].extensions["chart_rank"], 49);
        assert!(r.items[48].playable.is_none());
    }
}

#[test]
fn chart_history_rejects_current_fallback_invalid_period_ids_and_conflicting_availability() {
    let p = ChartPeriod::Day {
        date: "2026-01-01".into(),
    };
    let e = parse_period_tracks(&serde_json::to_vec(&detail(1)).unwrap(), "20", true, &p)
        .err()
        .unwrap();
    assert_eq!(e.details["reason"], "historical_period_not_returned");
    for case in 0..8 {
        let mut v = historical_detail(1);
        match case {
            0 => v["data"]["columnId"] = json!("21"),
            1 => v["data"]["periodColumnId"] = json!("0"),
            2 => v["data"]["periodColumnId"] = json!("02001"),
            3 => v["data"]["rankTypeList"] = json!(["0", "2"]),
            4 => v["data"]["rankTypeList"] = json!(["0", "1", "1"]),
            5 => v["data"]["dayRankUpdateTime"] = json!("20260229"),
            6 => v["data"]["weekRankUpdateTime"] = json!("2026-09-11"),
            _ => v["data"]["hasNextPage"] = json!(true),
        }
        assert!(
            parse_period_tracks(&serde_json::to_vec(&v).unwrap(), "20", true, &p).is_err(),
            "case {case}"
        );
    }
    assert!(
        parse_period_tracks(
            &serde_json::to_vec(&historical_detail(1)).unwrap(),
            "20",
            true,
            &ChartPeriod::Current
        )
        .is_err()
    );
}

#[test]
fn chart_history_metadata_keeps_unknown_period_codes_and_missing_dates_distinct() {
    let mut v = historical_detail(1);
    v["data"]["rankTypeList"] = json!(["0", "1", "2", "9"]);
    v["data"]["weekRankUpdateTime"] = json!("20241111");
    v["data"]["dayRankUpdateTime"] = Value::Null;
    let p = ChartPeriod::Week {
        date: "2026-09-14".into(),
    };
    let r = parse_period_tracks(&serde_json::to_vec(&v).unwrap(), "20", true, &p).unwrap();
    assert_eq!(
        r.extensions["upstream_rank_types"],
        json!(["0", "1", "2", "9"])
    );
    assert_eq!(
        r.extensions["available_period_kinds"],
        json!(["current", "day", "week"])
    );
    assert_eq!(r.extensions["period_start_dates"]["week"], "2024-11-11");
    assert!(r.extensions["period_start_dates"].get("day").is_none());
    assert_eq!(query(&p).unwrap(), ("2", "20260914".into()));
    v["data"]["rankTypeList"] = Value::Null;
    let r = parse_period_tracks(&serde_json::to_vec(&v).unwrap(), "20", true, &p).unwrap();
    assert!(!r.extensions.contains_key("available_period_kinds"));
}
