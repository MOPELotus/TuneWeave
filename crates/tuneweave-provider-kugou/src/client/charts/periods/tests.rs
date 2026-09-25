use super::*;
use crate::client::charts::tests::{info, tracks};

pub(crate) fn metadata(total: u64, period: u64) -> Value {
    let mut v = info(total);
    v["data"]["rank_cid"] = json!(period);
    v["data"]["ranktype"] = json!(2);
    v["data"]["zone"] = json!("tx6_gz_kmr");
    v
}
pub(crate) fn periods() -> Value {
    json!({"status":1,"errcode":0,"data":{"timestamp":1234567890,"rank_cid":7,"zone":"tx6_gz_kmr","vol_format":0,"info":[
        {"year":2026,"vols":[{"volid":7,"volname":"260期","voltime":"2026-09-17 08:30:00","outer_text":"2026-09-17 第260期"},{"volid":6,"volname":"259期","voltime":"2026-09-16 08:30:01"}]},
        {"year":2025,"vols":[{"volid":5,"volname":"365期","special_text":"Year end"}]}
    ]}})
}
fn snapshot() -> Snapshot {
    parse_info(&metadata(200, 7).to_string().into_bytes(), 42).unwrap()
}

#[test]
fn chart_period_catalogue_keeps_selectable_ids_years_labels_and_reported_coverage() {
    let p = parse_periods(&periods().to_string().into_bytes(), &snapshot()).unwrap();
    assert_eq!(p.items.len(), 3);
    assert_eq!(p.items[0].period, ChartPeriod::Id { id: "7".into() });
    assert_eq!(p.items[0].is_current, Some(true));
    assert_eq!(p.items[1].is_current, Some(false));
    assert_eq!(
        p.items[1].extensions["published_label"],
        "2026-09-16 08:30:01"
    );
    assert_eq!(p.items[2].year, Some(2025));
    assert_eq!(p.extensions["coverage_scope"], "upstream_returned_periods");
    assert_eq!(p.extensions["publication_timezone"], "unknown");
    let mut v = periods();
    v["data"]["info"] = json!([]);
    assert!(
        parse_periods(&v.to_string().into_bytes(), &snapshot())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn chart_period_catalogue_rejects_conflicting_binding_duplicates_bad_types_and_bounds() {
    for case in 0..10 {
        let mut v = periods();
        match case {
            0 => v["data"]["rank_cid"] = json!(8),
            1 => v["data"]["zone"] = json!("other"),
            2 => v["data"]["info"][1]["year"] = json!(2026),
            3 => v["data"]["info"][1]["vols"][0]["volid"] = json!(7),
            4 => v["data"]["info"][0]["vols"][0]["volid"] = json!(0),
            5 => v["data"]["info"][0]["vols"][0]["volid"] = json!("07"),
            6 => v["data"]["info"][0]["year"] = json!(10000),
            7 => v["data"]["info"][0]["vols"][0]["voltime"] = json!("x".repeat(513)),
            8 => {
                v["data"]["info"][0]["vols"] = json!(
                    (1..=10001)
                        .map(|i| json!({"volid":i,"volname":"period"}))
                        .collect::<Vec<_>>()
                )
            }
            _ => {
                v["data"]["info"] = json!(
                    (1..=101)
                        .map(|y| json!({"year":y,"vols":[]}))
                        .collect::<Vec<_>>()
                )
            }
        }
        assert!(
            parse_periods(&v.to_string().into_bytes(), &snapshot()).is_err(),
            "case {case}"
        );
    }
    for id in ["0", "07", "+7", "chart:7", "18446744073709551616"] {
        assert!(period_id(id).is_err());
    }
}

#[test]
fn chart_display_rank_gaps_keep_physical_positions_and_reject_bad_original_indices() {
    let mut v = tracks(2, 200);
    for row in v["data"]["songlist"].as_array_mut().unwrap() {
        let n = row["business"]["sort"].as_u64().unwrap();
        row["business"]["sort"] = json!(n + 1);
    }
    let (items, total) = parse_tracks(&v.to_string().into_bytes(), &snapshot(), 2, true).unwrap();
    assert_eq!(total, 200);
    assert_eq!(items[0].extensions["chart_position"], 100);
    assert_eq!(items[0].extensions["chart_rank"], 102);
    assert_eq!(items[99].extensions["chart_rank"], 201);
    assert_eq!(items[99].extensions["original_index"], 200);
    v["data"]["songlist"][99]["business"]["original_index"] = json!(201);
    assert!(parse_tracks(&v.to_string().into_bytes(), &snapshot(), 2, true).is_err());
    v["data"]["songlist"][99]["business"]["original_index"] = json!(200);
    v["data"]["songlist"][99]["business"]["sort"] = json!(0);
    assert!(parse_tracks(&v.to_string().into_bytes(), &snapshot(), 2, true).is_err());
}

#[test]
fn chart_known_missing_metadata_is_not_found_but_malformed_metadata_stays_error() {
    let missing = br#"{"status":1,"errcode":0,"data":{"timestamp":123}}"#;
    assert_eq!(
        parse_info(missing, 42).err().unwrap().code,
        ErrorCode::ResourceNotFound
    );
    for data in [
        json!({}),
        json!({"timestamp":123,"rankid":42}),
        json!({"timestamp":"invalid"}),
    ] {
        assert_eq!(
            parse_info(
                &json!({"status":1,"errcode":0,"data":data})
                    .to_string()
                    .into_bytes(),
                42
            )
            .err()
            .unwrap()
            .code,
            ErrorCode::UpstreamError
        );
    }
}
