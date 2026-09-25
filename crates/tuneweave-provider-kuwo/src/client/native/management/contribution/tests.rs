use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{management::items::tests::snapshot, tests as fixture},
};
use serde_json::Value;

pub(crate) const NAME: &str = "夜晚安静听的歌单";
pub(crate) const EDITED: &str = "新的夜晚安静歌单";
pub(crate) const PIC: &str = "https://img4.kuwo.cn/star/usercover/owned.jpg";
pub(crate) fn request(account: Option<&str>, edit: bool) -> PlaylistSubmissionRequest {
    PlaylistSubmissionRequest {
        recommendation: None,
        account: account.map(str::to_owned),
        name: edit.then(|| EDITED.into()),
        description: edit.then(|| "保存后投稿 + %20".into()),
        tags: edit.then(|| vec!["安静".into(), "夜晚".into()]),
    }
}
pub(crate) fn body(bytes: &[u8]) -> Value {
    let offset = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&bytes[offset..]).unwrap()
}
pub(crate) fn records(uid: &str, status: i32) -> Value {
    json!({"code":200,"data":{"list":[{"uid":uid,"id":101,"status":status,"name":NAME,"total":10}]}})
}
fn saved(uid: &str, edit: bool, online: bool) -> Vec<Vec<u8>> {
    let mut rows = snapshot(uid, &[11, 22, 22, 33, 44, 55, 66, 77, 88, 99]);
    for i in [0, 1, 4, 5] {
        let mut v = body(&rows[i]);
        let name = if edit { EDITED } else { NAME };
        let description = if edit {
            "保存后投稿 + %20"
        } else {
            "原始 + %20"
        };
        if let Some(p) = v.get_mut("plist").and_then(Value::as_array_mut) {
            p[0]["title"] = json!(name);
            p[0]["info"] = json!(description);
            p[0]["ispub"] = json!(true);
            p[0]["pic"] = json!(PIC);
        } else {
            v["sl_data"]["title"] = json!(name);
            v["sl_data"]["desc"] = json!(description);
            v["sl_data"]["pic"] = json!(PIC);
            v["sl_data"]["big_pic"] = json!(PIC);
            v["sl_data"]["igsl"] = json!(if online { "1" } else { "0" });
            if edit {
                v["sl_data"]["tag"] = json!("安静,夜晚");
                v["sl_data"]["tagid"] = json!("401,402");
            }
        }
        rows[i] = json_response(&v);
    }
    rows
}
pub(crate) fn flow(uid: &str, edit: bool) -> Vec<Vec<u8>> {
    let empty = json_response(&json!({"code":200,"data":{"list":[]}}));
    let mut rows = vec![json_response(&json!({"result":"ok"})), empty.clone(), empty];
    rows.extend(saved(uid, false, true));
    if edit {
        rows.push(json_response(&json!({"errcode":0,"pid":101})));
        rows.extend(saved(uid, true, true));
    }
    rows.push(json_response(&json!({"result":"success"})));
    rows.extend([
        json_response(&records(uid, 0)),
        json_response(&records(uid, 0)),
    ]);
    rows.extend(saved(uid, edit, false));
    rows
}
pub(crate) fn submit_at(edit: bool) -> usize {
    if edit { 16 } else { 9 }
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[tokio::test]
async fn native_contribution_saved_and_edited_published_lists_use_explicit_separate_writes() {
    for edit in [false, true] {
        let rows = flow("42", edit);
        let count = rows.len();
        let mut f = fixture::setup(rows).await;
        let result = f
            .client
            .native_submit_playlist(&credential(), "101", &request(None, edit))
            .await
            .unwrap();
        assert!(result.accepted);
        assert_eq!(result.metadata_updated, edit);
        assert_eq!(result.published, Some(false));
        assert_eq!(result.playlist.name, if edit { EDITED } else { NAME });
        assert_eq!(result.playlist.track_count, Some(10));
        assert_eq!(
            result.extensions["write_requests_dispatched"],
            if edit { 2 } else { 1 }
        );
        assert_eq!(result.extensions["records_correlated_to_request"], false);
        assert_eq!(
            result.records[0].review_status,
            tuneweave_core::PlaylistSubmissionStatus::Pending
        );
        assert!(result.records[0].published.is_none());
        let seen = fixture::requests(&mut f, count).await;
        let target = seen[submit_at(edit)].split_whitespace().nth(1).unwrap();
        let u = Url::parse(&format!("https://wapi.kuwo.cn{target}")).unwrap();
        let q = u.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(u.path(), SUBMIT_PATH);
        for (key, value) in [
            ("type", "user_songlist_up"),
            ("id", "101"),
            ("loginUid", "42"),
            ("sid", "selected-session"),
            ("loginSid", "selected-session"),
            ("uid", "1234567890"),
            ("apiVer", "1"),
            ("recWord", ""),
        ] {
            assert_eq!(q[key], value);
        }
        assert!(!seen[submit_at(edit)].to_lowercase().contains("\r\ncookie"));
        assert_eq!(
            seen.iter()
                .filter(|v| v.contains("type=user_songlist_up"))
                .count(),
            1
        );
        assert_eq!(
            seen.iter().filter(|v| v.starts_with("POST ")).count(),
            usize::from(edit)
        );
        if edit {
            let payload: Value =
                serde_json::from_str(seen[9].split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(
                payload,
                json!({"pid":101,"title":EDITED,"intro":"保存后投稿 + %20","tag":"安静,夜晚","pic":PIC,"ispub":true})
            );
        }
    }
}

#[tokio::test]
async fn native_contribution_acceptance_does_not_predict_review_or_publication_or_deduplicate_records()
 {
    for status in [0, 1, 2, 91] {
        for published in [false, true] {
            let mut rows = flow("42", false);
            let mut r = records("42", status);
            let duplicate = r["data"]["list"][0].clone();
            r["data"]["list"].as_array_mut().unwrap().push(duplicate);
            for i in [1, 2, 10, 11] {
                rows[i] = json_response(&r);
            }
            rows.splice(12.., saved("42", false, published));
            let mut f = fixture::setup(rows).await;
            let value = f
                .client
                .native_submit_playlist(&credential(), "101", &request(None, false))
                .await
                .unwrap();
            assert!(value.accepted);
            assert_eq!(value.records.len(), 2);
            assert_eq!(value.published, Some(published));
            assert_eq!(value.extensions["prior_record_count"], 2);
            assert_eq!(
                value.records[0].review_status,
                match status {
                    0 => tuneweave_core::PlaylistSubmissionStatus::Pending,
                    1 => tuneweave_core::PlaylistSubmissionStatus::Approved,
                    2 => tuneweave_core::PlaylistSubmissionStatus::Rejected,
                    _ => tuneweave_core::PlaylistSubmissionStatus::Unknown,
                }
            );
            assert!(value.records.iter().all(|r| r.published.is_none()));
            assert_eq!(
                fixture::requests(&mut f, 18)
                    .await
                    .iter()
                    .filter(|v| v.contains("user_songlist_up"))
                    .count(),
                1
            );
        }
    }
    let mut rows = flow("42", false);
    for i in [10, 11] {
        rows[i] = json_response(&json!({"data":{"list":[]}}));
    }
    let mut f = fixture::setup(rows).await;
    assert!(
        f.client
            .native_submit_playlist(&credential(), "101", &request(None, false))
            .await
            .unwrap()
            .records
            .is_empty()
    );
    fixture::requests(&mut f, 18).await;
}

#[tokio::test]
async fn native_contribution_validates_utf16_title_weight_and_public_metadata_before_writes() {
    for (name, valid) in [
        ("一".repeat(6), false),
        ("一".repeat(7), true),
        ("a".repeat(13), false),
        ("a".repeat(14), true),
        ("😀".repeat(7), true),
        ("一".repeat(20), true),
        ("一".repeat(21), false),
    ] {
        let mut rows = flow("42", true);
        if !valid {
            rows.truncate(9);
        }
        if valid {
            for i in [10, 11, 14, 15, 19, 20, 23, 24] {
                let mut b = body(&rows[i]);
                if b.get("plist").is_some() {
                    b["plist"][0]["title"] = json!(name);
                } else {
                    b["sl_data"]["title"] = json!(name);
                }
                rows[i] = json_response(&b);
            }
        }
        let count = rows.len();
        let mut f = fixture::setup(rows).await;
        let mut r = request(None, true);
        r.name = Some(name);
        let result = f
            .client
            .native_submit_playlist(&credential(), "101", &r)
            .await;
        if valid {
            assert!(result.unwrap().accepted);
        } else {
            let e = result.unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidRequest);
            assert!(e.details.get("write_requests_dispatched").is_none());
        }
        fixture::requests(&mut f, count).await;
    }
    for field in ["private", "cover", "tags", "description", "short"] {
        let mut rows = flow("42", false);
        rows.truncate(9);
        for i in [3, 4, 7, 8] {
            let mut b = body(&rows[i]);
            if let Some(p) = b.get_mut("plist").and_then(Value::as_array_mut) {
                match field {
                    "private" => p[0]["ispub"] = json!(false),
                    "cover" => p[0]["pic"] = json!(""),
                    "description" => p[0]["info"] = json!(""),
                    "short" => p[0]["title"] = json!("短"),
                    _ => {}
                }
            } else {
                match field {
                    "cover" => {
                        b["sl_data"]["pic"] = json!("");
                        b["sl_data"]["big_pic"] = json!("");
                    }
                    "tags" => {
                        b["sl_data"]["tag"] = json!("");
                        b["sl_data"].as_object_mut().unwrap().remove("tagid");
                    }
                    "description" => b["sl_data"]["desc"] = json!(""),
                    "short" => b["sl_data"]["title"] = json!("短"),
                    _ => {}
                }
            }
            rows[i] = json_response(&b);
        }
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_submit_playlist(&credential(), "101", &request(None, false))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest, "{field}");
        assert!(e.details.get("write_requests_dispatched").is_none());
        fixture::requests(&mut f, 9).await;
    }
}

#[tokio::test]
async fn native_contribution_rejection_transport_limits_and_partial_writes_are_not_retried() {
    let cases = vec![
        json_response(&json!({"result":"failed","reason":"selected-session"})),
        json_response(&json!({})),
        response(
            200,
            "application/json",
            "",
            br#"{"result":"success","result":"failed"}"#,
        ),
        response(401, "application/json", "", b"private"),
        response(403, "application/json", "", b"private"),
        response(429, "application/json", "Retry-After: 7\r\n", b"private"),
        response(
            302,
            "application/json",
            "Location: https://example.org/\r\n",
            b"",
        ),
        response(200, "text/html", "", b"success"),
        response(200, "application/json", "", &vec![b' '; 65537]),
    ];
    for edit in [false, true] {
        for bad in &cases {
            let at = submit_at(edit);
            let mut rows = flow("42", edit);
            rows.truncate(at + 1);
            rows[at] = bad.clone();
            let mut f = fixture::setup(rows).await;
            let e = f
                .client
                .native_submit_playlist(&credential(), "101", &request(None, edit))
                .await
                .unwrap_err();
            assert_eq!(
                e.details["write_requests_dispatched"],
                if edit { 2 } else { 1 }
            );
            assert_eq!(e.details["submission_outcome"], "unconfirmed");
            assert_eq!(
                e.details["playlist_write_outcome"],
                if edit { "confirmed" } else { "not_dispatched" }
            );
            assert!(!e.retryable);
            assert!(!format!("{} {}", e.message, e.details).contains("selected-session"));
            fixture::requests(&mut f, at + 1).await;
        }
    }
    let mut rows = flow("42", true);
    rows.truncate(10);
    rows[9] = json_response(&json!({"errcode":7}));
    let mut f = fixture::setup(rows).await;
    let e = f
        .client
        .native_submit_playlist(&credential(), "101", &request(None, true))
        .await
        .unwrap_err();
    assert_eq!(e.details["playlist_write_outcome"], "unconfirmed");
    assert_eq!(e.details["submission_outcome"], "not_dispatched");
    fixture::requests(&mut f, 10).await;
}

#[tokio::test]
async fn native_contribution_post_submit_drift_reports_accepted_without_false_completion() {
    for edit in [false, true] {
        for field in ["name", "tracks", "cover", "tags", "private", "records"] {
            let at = submit_at(edit);
            let mut rows = flow("42", edit);
            let start = at + 3;
            if field == "records" {
                rows[at + 2] = json_response(&records("42", 2));
                rows.truncate(at + 3);
            } else {
                for row in rows.iter_mut().skip(start).take(6) {
                    let mut b = body(row);
                    if let Some(p) = b.get_mut("plist").and_then(Value::as_array_mut) {
                        match field {
                            "name" => p[0]["title"] = json!("被其他客户端修改的歌单"),
                            "cover" => {
                                p[0]["pic"] = json!("https://img4.kuwo.cn/star/usercover/other.jpg")
                            }
                            "private" => p[0]["ispub"] = json!(false),
                            _ => {}
                        }
                    } else if b.get("sl_data").is_some() {
                        match field {
                            "name" => b["sl_data"]["title"] = json!("被其他客户端修改的歌单"),
                            "cover" => {
                                b["sl_data"]["pic"] =
                                    json!("https://img4.kuwo.cn/star/usercover/other.jpg");
                                b["sl_data"]["big_pic"] = b["sl_data"]["pic"].clone();
                            }
                            "tags" => b["sl_data"]["tag"] = json!("其他,标签"),
                            _ => {}
                        }
                    } else if field == "tracks" {
                        b["info"]["musiclist"].as_array_mut().unwrap().swap(0, 1);
                    }
                    *row = json_response(&b);
                }
            }
            let count = rows.len();
            let mut f = fixture::setup(rows).await;
            let e = f
                .client
                .native_submit_playlist(&credential(), "101", &request(None, edit))
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict, "{field}");
            assert_eq!(e.details["submission_outcome"], "accepted");
            assert_eq!(e.details["write_outcome"], "partial");
            fixture::requests(&mut f, count).await;
        }
    }
}

#[tokio::test]
async fn native_contribution_invalid_inputs_and_secret_reflection_do_not_start_network() {
    for (id, r) in [
        ("0", request(None, false)),
        ("101", request(Some("named"), false)),
        (
            "101",
            PlaylistSubmissionRequest {
                name: Some("".into()),
                ..Default::default()
            },
        ),
        (
            "101",
            PlaylistSubmissionRequest {
                name: Some("selected-session".into()),
                ..Default::default()
            },
        ),
        (
            "101",
            PlaylistSubmissionRequest {
                tags: Some(vec!["bad,tag".into()]),
                ..Default::default()
            },
        ),
    ] {
        let mut f = fixture::setup(Vec::new()).await;
        assert_eq!(
            f.client
                .native_submit_playlist(&credential(), id, &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        (&mut f.server).await.unwrap();
        assert!(f.seen.try_recv().is_err());
    }
}

#[tokio::test]
async fn native_contribution_fewer_than_ten_cloud_occurrences_rejects_before_either_write() {
    let mut rows = flow("42", true);
    rows.truncate(9);
    for row in rows.iter_mut().take(9).skip(3) {
        let mut b = body(row);
        if b.get("plist").is_some() {
            b["plist"][0]["musicnum"] = json!(9);
        } else if b.get("sl_data").is_some() {
            b["sl_data"]["total"] = json!(9);
        } else {
            b["info"]["musiclist"].as_array_mut().unwrap().pop();
        }
        *row = json_response(&b);
    }
    let mut f = fixture::setup(rows).await;
    let e = f
        .client
        .native_submit_playlist(&credential(), "101", &request(None, true))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    assert!(e.details.get("write_requests_dispatched").is_none());
    let seen = fixture::requests(&mut f, 9).await;
    assert!(
        seen.iter()
            .all(|s| !s.contains("user_songlist_up") && !s.starts_with("POST "))
    );
}

#[tokio::test]
async fn native_contribution_metadata_readback_conflict_stops_before_submission_and_preserves_partial_save()
 {
    for field in ["name", "tracks", "online"] {
        let mut rows = flow("42", true);
        rows.truncate(16);
        for row in rows.iter_mut().take(16).skip(10) {
            let mut b = body(row);
            if field == "name" {
                if b.get("plist").is_some() {
                    b["plist"][0]["title"] = json!("被其他客户端修改的歌单");
                } else if b.get("sl_data").is_some() {
                    b["sl_data"]["title"] = json!("被其他客户端修改的歌单");
                }
            } else if field == "online" && b.get("sl_data").is_some() {
                b["sl_data"]["igsl"] = json!("0");
            } else if field == "tracks" && b.get("info").is_some() {
                b["info"]["musiclist"].as_array_mut().unwrap().swap(0, 1);
            }
            *row = json_response(&b);
        }
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_submit_playlist(&credential(), "101", &request(None, true))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(e.details["playlist_write_outcome"], "unconfirmed");
        assert_eq!(e.details["submission_outcome"], "not_dispatched");
        let seen = fixture::requests(&mut f, 16).await;
        assert!(seen.iter().all(|s| !s.contains("user_songlist_up")));
    }
}
