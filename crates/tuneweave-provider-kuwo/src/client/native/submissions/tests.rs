use super::*;
use crate::client::catalog::tests::{json_response, response};
use crate::client::native::tests::{self as fixture, credential_fixture};

pub(crate) fn record(uid: &str, id: u64, status: i32) -> serde_json::Value {
    json!({"uid":uid,"id":id,"name":format!("Submission {id}"),"pic":"https://img4.kuwo.cn/cover.jpg",
        "total":12,"listencnt":100,"digest":10000,"status":status})
}
pub(crate) fn records(uid: &str, count: u64) -> Vec<serde_json::Value> {
    (0..count)
        .map(|i| record(uid, 101 + i, (i % 3) as i32))
        .collect()
}
pub(crate) fn body(items: &[serde_json::Value]) -> Vec<u8> {
    json_response(&json!({"code":200,"data":{"list":items}}))
}
pub(crate) fn flow(uid: &str, count: u64) -> Vec<Vec<u8>> {
    let items = records(uid, count);
    let mut bodies = vec![json_response(&json!({"result":"ok"}))];
    for _ in 0..2 {
        for page in items.chunks(6) {
            bodies.push(body(page));
        }
        if items.len() % 6 == 0 {
            bodies.push(body(&[]));
        }
    }
    bodies
}
fn credential() -> tuneweave_core::ProviderCredential {
    credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
fn input() -> KuwoNativeSessionInput {
    credential_fixture("42", "selected-session")
        .input()
        .unwrap()
}

#[tokio::test]
async fn native_submissions_complete_pagination_applies_window_and_separates_review_from_publication()
 {
    for (count, offset, limit) in [
        (0, 0, 10),
        (6, 0, 6),
        (8, 5, 2),
        (8, 7, 10),
        (8, 20, 10),
        (13, 5, 8),
    ] {
        let bodies = flow("42", count);
        let calls = bodies.len();
        let mut f = fixture::setup(bodies).await;
        let page = f
            .client
            .native_playlist_submissions(&credential(), &PageRequest::new(limit, offset))
            .await
            .unwrap();
        let expected = records("42", count)
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|v| v["id"].to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            page.items
                .iter()
                .map(|v| v.playlist_ref.id())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(page.pagination.total, Some(count));
        assert_eq!(
            page.pagination.has_more,
            (offset as u64 + page.items.len() as u64) < count
        );
        assert_eq!(
            page.pagination.extensions["upstream_pages_fetched"],
            calls - 1
        );
        for row in page.items {
            assert_eq!(row.owner_id, "42");
            assert_eq!(row.track_count, Some(12));
            assert_eq!(row.play_count, Some(100));
            assert_eq!(row.published, None);
            assert_eq!(
                row.review_status,
                match row.extensions["upstream_review_status"].as_i64().unwrap() {
                    0 => PlaylistSubmissionStatus::Pending,
                    1 => PlaylistSubmissionStatus::Approved,
                    _ => PlaylistSubmissionStatus::Rejected,
                }
            );
        }
        let seen = fixture::requests(&mut f, calls).await;
        let pages = (calls - 1) / 2;
        for (i, r) in seen.iter().skip(1).enumerate() {
            assert!(r.starts_with("GET /api/mobicase/playlist/userTgRecordList?"));
            assert!(r.contains(&format!(
                "loginUid=42&sid=selected-session&pn={}&rn=6",
                i % pages + 1
            )));
            assert!(!r.to_ascii_lowercase().contains("\r\ncookie"));
            assert!(!r.contains("user_songlist_up"));
        }
    }
}

#[test]
fn native_submissions_unknown_review_and_missing_metadata_are_not_fabricated() {
    for state in [None, Some(-1), Some(3), Some(i32::MAX)] {
        let mut row = json!({"uid":42,"id":i64::MAX});
        if let Some(code) = state {
            row["status"] = json!(code);
        }
        let rows = parse(
            json!({"data":{"list":[row]}}).to_string().as_bytes(),
            &input(),
        )
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.review_status, PlaylistSubmissionStatus::Unknown);
        assert_eq!(row.published, None);
        assert_eq!(row.extensions["upstream_review_status"], json!(state));
        assert_eq!(row.name, None);
        assert_eq!(row.track_count, None);
        assert_eq!(row.cover_url, None);
    }
}

#[test]
fn native_submissions_reject_bad_or_conflicting_data_without_echoing_secrets() {
    let good = json!({"code":200,"data":{"list":[record("42",101,0)]}});
    let mut cases = vec![
        json!({}),
        json!({"code":200}),
        json!({"data":{}}),
        json!({"data":{"list":null}}),
        json!({"data":{"list":{}}}),
        json!({"code":500,"data":{"list":[]}}),
        json!({"code":false,"data":{"list":[]}}),
        json!({"data":{"list":records("42",7)}}),
    ];
    for (key, value) in [
        ("uid", json!(43)),
        ("uid", json!("042")),
        ("id", json!(0)),
        ("id", json!("01")),
        ("id", json!(u64::MAX)),
        ("status", json!(true)),
        ("status", json!("01")),
        ("status", json!(1.1)),
        ("status", json!(2147483648u64)),
        ("total", json!(-1)),
        ("listencnt", json!(1.5)),
        ("digest", json!("-1")),
        ("name", json!("selected-session")),
        ("name", json!("x\ny")),
        ("name", json!("x".repeat(1025))),
        ("pic", json!("https://evil.test/image.jpg")),
        ("pic", json!("https://img4.kuwo.cn/selected-session.jpg")),
    ] {
        let mut v = good.clone();
        v["data"]["list"][0][key] = value;
        cases.push(v);
    }
    for v in cases {
        let e = parse(v.to_string().as_bytes(), &input()).unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError, "{v}");
        assert!(!e.to_string().contains("selected-session"));
    }
    for bytes in [
        br#"{"data":{"list":[]},"data":{"list":[]}}"#.as_slice(),
        br#"{"data":{"list":[{"uid":42,"id":101,"id":102}]}}"#.as_slice(),
    ] {
        assert!(parse(bytes, &input()).is_err());
    }
    assert_eq!(
        parse(br#"{"code":-1001,"msg":"auth fail"}"#, &input())
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn native_submissions_duplicate_records_are_preserved_and_repeated_full_pages_rejected() {
    let row = record("42", 101, 0);
    let changed = record("42", 101, 2);
    let page = body(&[row.clone(), row, changed]);
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        page.clone(),
        page,
    ])
    .await;
    let result = f
        .client
        .native_playlist_submissions(&credential(), &PageRequest::new(10, 0))
        .await
        .unwrap();
    assert_eq!(result.items.len(), 3);
    assert_eq!(result.items[0], result.items[1]);
    assert_ne!(result.items[1].review_status, result.items[2].review_status);
    fixture::requests(&mut f, 3).await;
    let mut b = flow("42", 8);
    b[2] = b[1].clone();
    b.truncate(3);
    let mut f = fixture::setup(b).await;
    assert_eq!(
        f.client
            .native_playlist_submissions(&credential(), &PageRequest::new(10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_submissions_second_read_changes_never_return_partial_pages() {
    for key in [
        "uid",
        "id",
        "status",
        "name",
        "total",
        "listencnt",
        "digest",
        "pic",
    ] {
        let mut b = flow("42", 8);
        let mut items = records("42", 8);
        items[0][key] = match key {
            "uid" => json!(43),
            "id" => json!(999),
            "status" => json!(1),
            "name" => json!("Changed"),
            "pic" => json!("https://img4.kuwo.cn/new.jpg"),
            _ => json!(999),
        };
        b[3] = body(&items[..6]);
        let calls = if key == "uid" { 4 } else { 5 };
        b.truncate(calls);
        let mut f = fixture::setup(b).await;
        let e = f
            .client
            .native_playlist_submissions(&credential(), &PageRequest::new(1, 7))
            .await
            .unwrap_err();
        assert_eq!(
            e.code,
            if key == "uid" {
                ErrorCode::UpstreamError
            } else {
                ErrorCode::Conflict
            }
        );
        fixture::requests(&mut f, calls).await;
    }
}

#[tokio::test]
async fn native_submissions_transport_and_resource_budgets_are_bounded() {
    for (reply, code) in [
        (
            response(401, "application/json", "", b"{}"),
            ErrorCode::AuthenticationRequired,
        ),
        (
            response(403, "application/json", "", b"{}"),
            ErrorCode::PermissionDenied,
        ),
        (
            response(429, "application/json", "", b"{}"),
            ErrorCode::RateLimited,
        ),
        (
            response(302, "application/json", "", b"{}"),
            ErrorCode::UpstreamError,
        ),
        (
            response(200, "text/html", "", b"{}"),
            ErrorCode::UpstreamError,
        ),
        (
            json_response(&json!({"code":-1001})),
            ErrorCode::AuthenticationRequired,
        ),
    ] {
        let mut f = fixture::setup(vec![json_response(&json!({"result":"ok"})), reply]).await;
        assert_eq!(
            f.client
                .native_playlist_submissions(&credential(), &PageRequest::new(10, 0))
                .await
                .unwrap_err()
                .code,
            code
        );
        fixture::requests(&mut f, 2).await;
    }
    for (limits, calls) in [
        (
            Limits {
                pages: 1,
                ..Limits::default()
            },
            2,
        ),
        (
            Limits {
                response_bytes: 10,
                ..Limits::default()
            },
            2,
        ),
        (
            Limits {
                total_bytes: 64,
                ..Limits::default()
            },
            2,
        ),
    ] {
        let mut b = flow("42", 8);
        b.truncate(calls);
        let mut f = fixture::setup(b).await;
        f.client.native_submission_limits = Some(limits);
        assert_eq!(
            f.client
                .native_playlist_submissions(&credential(), &PageRequest::new(10, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        fixture::requests(&mut f, calls).await;
    }
}

#[tokio::test]
async fn native_submissions_invalid_pagination_and_sdk_account_never_send() {
    let mut f = fixture::setup(vec![]).await;
    for mut r in [
        PageRequest::new(0, 0),
        PageRequest::new(101, 0),
        PageRequest::new(1, u32::MAX),
        PageRequest::new(1, 0),
    ] {
        if r.limit == 1 && r.offset == 0 {
            r.account = Some("personal".into());
        }
        assert_eq!(
            f.client
                .native_playlist_submissions(&credential(), &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    fixture::requests(&mut f, 0).await;
}

#[tokio::test]
async fn native_submissions_full_page_limit_requires_a_short_last_page() {
    for (count, allowed, calls) in [(1535, true, 513), (1536, false, 257)] {
        let mut bodies = flow("42", count);
        bodies.truncate(calls);
        let mut f = fixture::setup(bodies).await;
        let result = f
            .client
            .native_playlist_submissions(&credential(), &PageRequest::new(2, 1533))
            .await;
        if allowed {
            let page = result.unwrap();
            assert_eq!(page.pagination.total, Some(1535));
            assert_eq!(page.items.len(), 2);
            assert_eq!(page.items[1].playlist_ref.id(), "1635");
            assert!(!page.pagination.has_more);
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::UpstreamError);
        }
        fixture::requests(&mut f, calls).await;
    }
}

#[tokio::test]
async fn native_submissions_cumulative_limit_spans_both_reads_and_counts_chunked_bodies() {
    let items = records("42", 8);
    let first_bytes = serde_json::to_vec(&json!({"code":200,"data":{"list":&items[..6]}})).unwrap();
    let last_bytes = serde_json::to_vec(&json!({"code":200,"data":{"list":&items[6..]}})).unwrap();
    for (total_bytes, calls) in [
        (first_bytes.len() + last_bytes.len(), 3),
        (first_bytes.len() * 2 + last_bytes.len() - 1, 4),
    ] {
        let mut bodies = flow("42", 8);
        bodies.truncate(calls);
        let mut f = fixture::setup(bodies).await;
        f.client.native_submission_limits = Some(Limits {
            total_bytes,
            ..Limits::default()
        });
        assert_eq!(
            f.client
                .native_playlist_submissions(&credential(), &PageRequest::new(2, 5))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        fixture::requests(&mut f, calls).await;
    }
    let chunked=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",first_bytes.len(),String::from_utf8(first_bytes.clone()).unwrap()).into_bytes();
    let mut f = fixture::setup(vec![json_response(&json!({"result":"ok"})), chunked]).await;
    f.client.native_submission_limits = Some(Limits {
        response_bytes: first_bytes.len() - 1,
        ..Limits::default()
    });
    assert_eq!(
        f.client
            .native_playlist_submissions(&credential(), &PageRequest::new(2, 5))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    fixture::requests(&mut f, 2).await;
}
