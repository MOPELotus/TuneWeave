use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::contribution::tests::{body, flow as submission_flow},
        tests as fixture,
    },
};
use serde_json::Value;

pub(crate) fn request(account: Option<&str>) -> PlaylistSubmissionRecordDeleteRequest {
    PlaylistSubmissionRecordDeleteRequest {
        account: account.map(str::to_owned),
    }
}
pub(crate) fn delete_at(present: bool) -> usize {
    if present { 10 } else { 5 }
}
fn record(uid: &str, id: u64, status: i32) -> Value {
    json!({"uid":uid,"id":id,"status":status,"name":"历史歌单","total":10})
}
fn scan(rows: &[Value]) -> Vec<Vec<u8>> {
    let mut pages: Vec<_> = rows
        .chunks(6)
        .map(|v| json_response(&json!({"code":200,"data":{"list":v}})))
        .collect();
    if rows.len() % 6 == 0 {
        pages.push(json_response(&json!({"data":{"list":[]}})));
    }
    pages
}
fn records(rows: &[Value]) -> Vec<Vec<u8>> {
    let mut r = scan(rows);
    r.extend(scan(rows));
    r
}
pub(crate) fn observation(uid: &str, present: bool) -> Vec<Vec<u8>> {
    if present {
        let snapshot = submission_flow(uid, false)[3..9].to_vec();
        let mut r = vec![snapshot[0].clone()];
        r.extend(snapshot);
        r
    } else {
        vec![json_response(&json!({"errcode":0,"plist":[]})); 2]
    }
}
pub(crate) fn flow(uid: &str, present: bool) -> Vec<Vec<u8>> {
    custom_flow(
        uid,
        present,
        &[
            record(uid, 101, 0),
            record(uid, 102, 1),
            record(uid, 101, 2),
        ],
        true,
    )
}
fn custom_flow(uid: &str, present: bool, before: &[Value], write: bool) -> Vec<Vec<u8>> {
    let mut r = vec![json_response(&json!({"result":"ok"}))];
    r.extend(records(before));
    r.extend(observation(uid, present));
    if write {
        r.push(json_response(&json!({"data":{"result":"success"}})));
    }
    let after: Vec<_> = before.iter().filter(|v| v["id"] != 101).cloned().collect();
    r.extend(records(&after));
    r.extend(observation(uid, present));
    r
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[tokio::test]
async fn native_submission_delete_keeps_duplicates_counts_and_independent_playlist_observation() {
    for present in [false, true] {
        let rows = flow("42", present);
        let count = rows.len();
        let mut f = fixture::setup(rows).await;
        let result = f
            .client
            .native_delete_playlist_submission_records(&credential(), "101", &request(None))
            .await
            .unwrap();
        assert!(result.confirmed && result.changed);
        assert_eq!(result.removed_records, 2);
        assert_eq!(result.owned_playlist_present, present);
        assert_eq!(result.playlist.is_some(), present);
        assert_eq!(result.published, present.then_some(true));
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(
            result.extensions["playlist_observation_scope"],
            "ordinary_created_directory"
        );
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("selected-session") && !encoded.contains("withdrawn"));
        let seen = fixture::requests(&mut f, count).await;
        let write = &seen[delete_at(present)];
        assert!(write.starts_with("POST "));
        let url = Url::parse(&format!(
            "https://wapi.kuwo.cn{}",
            write.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        assert_eq!(url.path(), DELETE_PATH);
        assert_eq!(
            url.query_pairs().collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("loginUid".into(), "42".into()),
                ("sid".into(), "selected-session".into())
            ])
        );
        assert_eq!(write.split_once("\r\n\r\n").unwrap().1, "ids=101");
        assert!(
            write
                .to_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        assert!(!write.to_lowercase().contains("\r\ncookie"));
        assert_eq!(seen.iter().filter(|v| v.starts_with("POST ")).count(), 1);
        assert!(
            seen.iter()
                .all(|v| !v.contains("pl3_delete") && !v.contains("user_songlist_up"))
        );
    }
}

#[tokio::test]
async fn native_submission_delete_absent_records_are_a_verified_noop_even_with_an_orphaned_playlist()
 {
    for present in [false, true] {
        let rows = custom_flow("42", present, &[record("42", 102, 1)], false);
        let count = rows.len();
        let mut f = fixture::setup(rows).await;
        let result = f
            .client
            .native_delete_playlist_submission_records(&credential(), "101", &request(None))
            .await
            .unwrap();
        assert!(result.confirmed);
        assert!(!result.changed);
        assert_eq!(result.removed_records, 0);
        assert_eq!(result.extensions["record_delete_outcome"], "already_absent");
        assert_eq!(result.extensions["write_requests_dispatched"], 0);
        assert!(
            fixture::requests(&mut f, count)
                .await
                .iter()
                .all(|v| !v.starts_with("POST "))
        );
    }
}

#[tokio::test]
async fn native_submission_delete_multi_page_history_preserves_all_other_rows_and_order() {
    let before: Vec<_> = (0..19)
        .map(|i| record("42", if i % 4 == 0 { 101 } else { 200 + i }, i as i32))
        .collect();
    let rows = custom_flow("42", false, &before, true);
    let count = rows.len();
    let mut f = fixture::setup(rows).await;
    let value = f
        .client
        .native_delete_playlist_submission_records(&credential(), "101", &request(None))
        .await
        .unwrap();
    assert_eq!(value.removed_records, 5);
    let seen = fixture::requests(&mut f, count).await;
    assert_eq!(seen.iter().filter(|v| v.starts_with("POST ")).count(), 1);
    assert!(seen.iter().any(|v| v.contains("pn=4&rn=6")));
}

#[tokio::test]
async fn native_submission_delete_publication_is_an_observation_and_not_a_withdrawal_receipt() {
    for (before, after) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut rows = flow("42", true);
        for (start, end, online) in [(3, 10, before), (13, 20, after)] {
            for row in &mut rows[start..end] {
                let mut b = body(row);
                if let Some(detail) = b.get_mut("sl_data") {
                    detail["igsl"] = json!(if online { "1" } else { "0" });
                }
                *row = json_response(&b);
            }
        }
        let mut f = fixture::setup(rows).await;
        let value = f
            .client
            .native_delete_playlist_submission_records(&credential(), "101", &request(None))
            .await
            .unwrap();
        assert_eq!(value.published, Some(after));
        assert_eq!(value.extensions["published_before"], before);
        assert_eq!(value.extensions["publication_changed"], before != after);
        fixture::requests(&mut f, 20).await;
    }
}

#[tokio::test]
async fn native_submission_delete_bad_ack_status_size_or_type_is_not_retried_or_reported_as_confirmed()
 {
    let mut bad: Vec<_> = [
        r#"{}"#,
        r#"{"result":"success"}"#,
        r#"{"data":{"result":"SUCCESS"}}"#,
        r#"{"data":{"result":"failed","reason":"selected-session"}}"#,
        r#"{"data":{"result":"success","result":"success"}}"#,
        r#"{"code":500,"data":{"result":"success"}}"#,
        r#"{"code":-1001}"#,
    ]
    .iter()
    .map(|v| response(200, "application/json", "", v.as_bytes()))
    .collect();
    bad.extend([
        response(200, "text/html", "", b"private"),
        response(200, "application/json", "", &vec![b' '; ACK_LIMIT + 1]),
    ]);
    for status in [401, 403, 429, 302, 500] {
        bad.push(response(
            status,
            "application/json",
            "Location: https://untrusted.invalid/\r\n",
            b"private",
        ));
    }
    for ack in bad {
        let mut rows = flow("42", false);
        rows.truncate(6);
        rows[5] = ack;
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_delete_playlist_submission_records(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(e.details["record_delete_outcome"], "unconfirmed");
        assert_eq!(e.details["automatic_retry"], false);
        assert!(!e.retryable);
        assert!(!format!("{}{}", e.message, e.details).contains("selected-session"));
        assert_eq!(
            fixture::requests(&mut f, 6)
                .await
                .iter()
                .filter(|v| v.starts_with("POST "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn native_submission_delete_partial_removal_or_unrelated_history_change_cannot_confirm_success()
 {
    for change in ["remaining", "other", "new", "uid", "secret"] {
        let mut rows = flow("42", false);
        rows.truncate(8);
        let mut after = vec![record("42", 102, 1)];
        match change {
            "remaining" => after.push(record("42", 101, 2)),
            "other" => after[0]["status"] = json!(2),
            "new" => after.push(record("42", 103, 0)),
            "uid" => after[0]["uid"] = json!("43"),
            _ => after[0]["name"] = json!("selected-session"),
        }
        rows[6] = scan(&after)[0].clone();
        rows[7] = rows[6].clone();
        if matches!(change, "uid" | "secret") {
            rows.truncate(7);
        }
        let count = rows.len();
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_delete_playlist_submission_records(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(
            e.details["record_delete_outcome"],
            if matches!(change, "other" | "new") {
                "confirmed"
            } else {
                "acknowledged"
            }
        );
        assert!(!format!("{}{}", e.message, e.details).contains("selected-session"));
        fixture::requests(&mut f, count).await;
    }
}

#[tokio::test]
async fn native_submission_delete_playlist_content_or_presence_drift_is_reported_after_confirmed_record_removal()
 {
    for change in ["name", "tracks", "visibility", "gone", "appeared"] {
        let present = change != "appeared";
        let mut rows = flow("42", present);
        let start = delete_at(present) + 3;
        if change == "gone" || change == "appeared" {
            rows.splice(start.., observation("42", !present));
        } else {
            for row in &mut rows[start..] {
                let mut b = body(row);
                if change == "name" {
                    if let Some(p) = b.get_mut("plist") {
                        p[0]["title"] = json!("外部修改的歌单");
                    }
                    if let Some(p) = b.get_mut("sl_data") {
                        p["title"] = json!("外部修改的歌单");
                    }
                } else if change == "visibility" {
                    if let Some(p) = b.get_mut("plist") {
                        p[0]["ispub"] = json!(false);
                    }
                    if let Some(p) = b.get_mut("sl_data") {
                        p["isPrivate"] = json!(true);
                    }
                } else if let Some(p) = b.get_mut("info") {
                    p["musiclist"].as_array_mut().unwrap().swap(0, 1);
                }
                *row = json_response(&b);
            }
        }
        let count = rows.len();
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_delete_playlist_submission_records(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.details["record_delete_outcome"], "confirmed", "{change}");
        assert_eq!(e.details["write_outcome"], "partial");
        if matches!(change, "gone" | "appeared") {
            assert_eq!(e.details["owned_playlist_present_before"], present);
            assert_eq!(e.details["owned_playlist_present_after"], !present);
        }
        fixture::requests(&mut f, count).await;
    }
}

#[tokio::test]
async fn native_submission_delete_invalid_ids_and_sdk_aliases_do_not_start_network() {
    for (id, account) in [
        ("0", None),
        ("01", None),
        ("101,102", None),
        ("9223372036854775808", None),
        ("101", Some("named")),
    ] {
        let mut f = fixture::setup(vec![]).await;
        assert_eq!(
            f.client
                .native_delete_playlist_submission_records(&credential(), id, &request(account))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        (&mut f.server).await.unwrap();
        assert!(f.seen.try_recv().is_err());
    }
}
