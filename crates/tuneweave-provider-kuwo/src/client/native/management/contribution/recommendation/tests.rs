use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::contribution::tests::{self as base, body},
        profile::tests as profile_fixture,
        tests as fixture,
    },
};
use serde_json::Value;

pub(crate) const RECOMMENDATION: &str = "好歌🎵";
pub(crate) fn request(account: Option<&str>, edit: bool) -> PlaylistSubmissionRequest {
    let mut r = base::request(account, edit);
    r.recommendation = Some(RECOMMENDATION.into());
    r
}
fn checked(id: u64, fields: &[(&str, &str)]) -> Vec<u8> {
    response(200,"text/html; charset=utf-8","",&serde_json::to_vec(&json!({"playlistid":id,"result":"ok",
        "list":fields.iter().map(|(kind,content)|json!({"type":kind,"content":content,"issensitive":false})).collect::<Vec<_>>()})).unwrap())
}
pub(crate) fn flow(uid: &str, edit: bool) -> Vec<Vec<u8>> {
    let original = base::flow(uid, edit);
    let mut rows = original[..9].to_vec();
    rows.extend([
        profile_fixture::reply(uid),
        checked(0, &[("intro", RECOMMENDATION)]),
        checked(
            101,
            &[
                ("name", if edit { base::EDITED } else { base::NAME }),
                (
                    "intro",
                    if edit {
                        "保存后投稿 + %20"
                    } else {
                        "原始 + %20"
                    },
                ),
            ],
        ),
        json_response(&json!({"opret":"ok"})),
    ]);
    if edit {
        rows.extend_from_slice(&original[10..]);
    } else {
        rows.extend_from_slice(&original[3..9]);
        rows.extend_from_slice(&original[9..]);
    }
    assert_eq!(rows.len(), 28);
    rows
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[tokio::test]
async fn native_recommendation_private_name_keyword_bindings_and_separate_save_precede_submit() {
    for edit in [false, true] {
        let mut f = fixture::setup(flow("42", edit)).await;
        let result = f
            .client
            .native_submit_playlist(&credential(), "101", &request(None, edit))
            .await
            .unwrap();
        assert!(result.accepted && result.metadata_updated);
        assert_eq!(result.published, Some(false));
        assert_eq!(result.extensions["recommendation_submitted"], true);
        assert_eq!(result.extensions["write_requests_dispatched"], 2);
        let encoded = serde_json::to_string(&result).unwrap();
        for secret in [
            "private-login-name",
            "private-answer",
            "private-mail",
            "selected-session",
        ] {
            assert!(!encoded.contains(secret));
        }
        let seen = fixture::requests(&mut f, 28).await;
        assert!(seen[9].starts_with("GET /userinfo/lua_get_user_and_follow?"));
        for (at, id, fields) in [
            (10, 0, vec![("intro", RECOMMENDATION)]),
            (
                11,
                101,
                vec![
                    ("name", if edit { base::EDITED } else { base::NAME }),
                    (
                        "intro",
                        if edit {
                            "保存后投稿 + %20"
                        } else {
                            "原始 + %20"
                        },
                    ),
                ],
            ),
        ] {
            assert!(
                seen[at].starts_with("POST /pl.svc?op=verifykeyword&encode=utf-8&plat=ar&uid=42 ")
            );
            let value: Value =
                serde_json::from_str(seen[at].split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(
                value,
                json!({"playlistid":id,"list":fields.into_iter().map(|(kind,content)|json!({"type":kind,"content":content})).collect::<Vec<_>>() })
            );
        }
        assert!(seen[12].starts_with(
            "POST /pl.svc?op=updatelistinfo&encode=utf-8&pid=101&uid=42&sid=selected-session "
        ));
        let value: Value =
            serde_json::from_str(seen[12].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            value,
            json!({"name":if edit {base::EDITED} else {base::NAME},"info":if edit {"保存后投稿 + %20"} else {"原始 + %20"},"tag":if edit {"安静,夜晚"} else {"流行,安静"},"pic":base::PIC,"ispub":true,"uname":"private-login-name"})
        );
        for at in [10, 11, 12] {
            assert!(
                seen[at]
                    .to_lowercase()
                    .contains("content-type: application/x-www-form-urlencoded")
            );
            assert!(!seen[at].to_lowercase().contains("\r\ncookie"));
        }
        let url = Url::parse(&format!(
            "https://wapi.kuwo.cn{}",
            seen[19].split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "recWord").unwrap().1,
            RECOMMENDATION
        );
        assert!(seen.iter().all(|v| !v.contains("pl3_editlist")));
        assert_eq!(
            seen.iter()
                .filter(|v| v.contains("user_songlist_up"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn native_recommendation_utf16_whitespace_and_legacy_title_limits_precede_writes() {
    for text in ["", " ", "hello!", "😀😀😀", " good", "good ", "a\nb"] {
        let mut r = request(None, false);
        r.recommendation = Some(text.into());
        let mut f = fixture::setup(vec![]).await;
        assert_eq!(
            f.client
                .native_submit_playlist(&credential(), "101", &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        (&mut f.server).await.unwrap();
        assert!(f.seen.try_recv().is_err());
    }
    for text in ["a", "12345", "😀😀a", "一二三四五"] {
        let mut r = request(None, false);
        r.recommendation = Some(text.into());
        let mut rows = flow("42", false);
        rows[10] = checked(0, &[("intro", text)]);
        let mut f = fixture::setup(rows).await;
        assert!(
            f.client
                .native_submit_playlist(&credential(), "101", &r)
                .await
                .unwrap()
                .accepted
        );
        fixture::requests(&mut f, 28).await;
    }
    for title in ["一".repeat(6), "一".repeat(17), format!(" {}", base::NAME)] {
        let mut r = request(None, true);
        r.name = Some(title);
        let mut rows = flow("42", true);
        rows.truncate(9);
        let mut f = fixture::setup(rows).await;
        assert_eq!(
            f.client
                .native_submit_playlist(&credential(), "101", &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        fixture::requests(&mut f, 9).await;
    }
}

#[tokio::test]
async fn native_recommendation_private_name_never_falls_back_to_nickname_or_leaks_on_error() {
    let mut samples = vec![
        json!({"status":200,"info":[]}),
        json!({"status":500,"info":[{"UID":42,"NAME":"private-login-name"}]}),
        json!({"status":200,"info":[{"UID":43,"NAME":"private-login-name"}]}),
    ];
    for value in [
        Value::Null,
        json!(3),
        json!(""),
        json!(" "),
        json!("a\nb"),
        json!("x".repeat(1025)),
        json!("selected-session"),
    ] {
        let mut b = profile_fixture::body("42");
        b["info"][0]["NAME"] = value;
        samples.push(b);
    }
    for b in samples {
        let mut rows = flow("42", false);
        rows.truncate(10);
        rows[9] = fixture::encrypted(&b);
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_submit_playlist(&credential(), "101", &request(None, false))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(e.details.get("write_requests_dispatched").is_none());
        assert!(!format!("{}{}", e.message, e.details).contains("private-login-name"));
        fixture::requests(&mut f, 10).await;
    }
    let input = credential::NativeCredential::parse(&credential())
        .unwrap()
        .input()
        .unwrap();
    let author = parse_author(
        &serde_json::to_vec(&profile_fixture::body("42")).unwrap(),
        &input,
    )
    .unwrap();
    assert_eq!(format!("{author:?}"), "Author([redacted])");
    assert!(
        parse_author(
            br#"{"status":200,"info":[{"UID":42,"NAME":"a","NAME":"b"}]}"#,
            &input
        )
        .is_err()
    );
}

#[tokio::test]
async fn native_recommendation_keyword_checks_require_exact_complete_explicit_results_before_saving()
 {
    for boundary in [10, 11] {
        for change in [
            "missing",
            "duplicate",
            "wrong-id",
            "wrong-content",
            "wrong-type",
            "unknown",
            "sensitive",
            "failure",
        ] {
            let mut rows = flow("42", false);
            rows.truncate(boundary + 1);
            let mut b = body(&rows[boundary]);
            match change {
                "missing" => {
                    b["list"].as_array_mut().unwrap().pop();
                }
                "duplicate" => {
                    let row = b["list"][0].clone();
                    b["list"].as_array_mut().unwrap().push(row);
                }
                "wrong-id" => b["playlistid"] = json!(99),
                "wrong-content" => b["list"][0]["content"] = json!("private-login-name"),
                "wrong-type" => b["list"][0]["type"] = json!("recWord"),
                "unknown" => {
                    b["list"][0].as_object_mut().unwrap().remove("issensitive");
                }
                "sensitive" => b["list"][0]["issensitive"] = json!(true),
                _ => b["result"] = json!("failed"),
            }
            rows[boundary] = json_response(&b);
            let mut f = fixture::setup(rows).await;
            let e = f
                .client
                .native_submit_playlist(&credential(), "101", &request(None, false))
                .await
                .unwrap_err();
            assert_eq!(
                e.code,
                if change == "sensitive" {
                    ErrorCode::PermissionDenied
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert!(e.details.get("write_requests_dispatched").is_none());
            assert!(!format!("{}{}", e.message, e.details).contains("private-login-name"));
            assert!(
                fixture::requests(&mut f, boundary + 1)
                    .await
                    .iter()
                    .all(|v| !v.contains("updatelistinfo") && !v.contains("user_songlist_up"))
            );
        }
    }
}

#[tokio::test]
async fn native_recommendation_save_ack_is_not_submission_and_partial_writes_are_not_retried() {
    for bad in [
        json!({"errcode":0}),
        json!({"opret":"failed","reason":"private-login-name"}),
        json!({"opret":0}),
    ] {
        let mut rows = flow("42", false);
        rows.truncate(13);
        rows[12] = json_response(&bad);
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_submit_playlist(&credential(), "101", &request(None, false))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(e.details["playlist_write_outcome"], "unconfirmed");
        assert_eq!(e.details["submission_outcome"], "not_dispatched");
        assert!(!e.retryable);
        assert_eq!(
            fixture::requests(&mut f, 13)
                .await
                .iter()
                .filter(|v| v.contains("updatelistinfo"))
                .count(),
            1
        );
    }
    let mut rows = flow("42", false);
    rows.truncate(20);
    rows[19] = response(401, "application/json", "", b"private-login-name");
    let mut f = fixture::setup(rows).await;
    let e = f
        .client
        .native_submit_playlist(&credential(), "101", &request(None, false))
        .await
        .unwrap_err();
    assert_eq!(e.details["write_requests_dispatched"], 2);
    assert_eq!(e.details["playlist_write_outcome"], "confirmed");
    assert_eq!(e.details["submission_outcome"], "unconfirmed");
    assert_eq!(e.details["write_outcome"], "partial");
    fixture::requests(&mut f, 20).await;
}

#[tokio::test]
async fn native_recommendation_transport_failures_and_response_limits_do_not_skip_preflight() {
    for boundary in [10, 11, 12] {
        let mut responses = vec![
            response(200, "image/png", "", b"private"),
            response(200, "text/html", "", b"<html>private</html>"),
            response(200, "application/json", "", &vec![b' '; ACK_LIMIT + 1]),
        ];
        for status in [401, 403, 429, 302, 500] {
            responses.push(response(
                status,
                "application/json",
                "Location: https://other.invalid/\r\n",
                b"private",
            ));
        }
        for failed in responses {
            let mut rows = flow("42", false);
            rows.truncate(boundary + 1);
            rows[boundary] = failed;
            let mut f = fixture::setup(rows).await;
            let e = f
                .client
                .native_submit_playlist(&credential(), "101", &request(None, false))
                .await
                .unwrap_err();
            assert_eq!(
                e.details.get("write_requests_dispatched").is_some(),
                boundary == 12
            );
            fixture::requests(&mut f, boundary + 1).await;
        }
    }
}

#[tokio::test]
async fn native_recommendation_legacy_title_edges_and_independent_publication_observation() {
    for title in [
        "一".repeat(7),
        "一".repeat(16),
        "😀".repeat(16),
        "a".repeat(32),
    ] {
        let mut r = request(None, true);
        r.name = Some(title.clone());
        let mut rows = flow("42", true);
        rows[11] = checked(101, &[("name", &title), ("intro", "保存后投稿 + %20")]);
        for at in [13, 14, 17, 18, 22, 23, 26, 27] {
            let mut value = body(&rows[at]);
            if value.get("plist").is_some() {
                value["plist"][0]["title"] = json!(title);
            } else {
                value["sl_data"]["title"] = json!(title);
                // Saving can change publication before the explicit submit ACK.
                value["sl_data"]["igsl"] = json!("0");
            }
            rows[at] = json_response(&value);
        }
        let mut f = fixture::setup(rows).await;
        let value = f
            .client
            .native_submit_playlist(&credential(), "101", &r)
            .await
            .unwrap();
        assert_eq!(value.playlist.name, title);
        assert_eq!(value.published, Some(false));
        assert!(value.accepted && value.metadata_updated);
        fixture::requests(&mut f, 28).await;
    }
}

#[tokio::test]
async fn native_recommendation_saved_state_drift_stops_before_submission_without_claiming_rollback()
{
    for change in ["name", "tags", "cover", "tracks"] {
        let mut rows = flow("42", false);
        rows.truncate(19);
        for (at, row) in rows.iter_mut().enumerate().take(19).skip(13) {
            let mut value = body(row);
            match change {
                "name" if [13, 18].contains(&at) => {
                    value["plist"][0]["title"] = json!("意外改变的歌单名称")
                }
                "name" if [14, 17].contains(&at) => {
                    value["sl_data"]["title"] = json!("意外改变的歌单名称")
                }
                "tags" if [14, 17].contains(&at) => {
                    value["sl_data"]["tag"] = json!("意外标签,安静")
                }
                "cover" if [13, 18].contains(&at) => {
                    value["plist"][0]["pic"] =
                        json!("https://img4.kuwo.cn/star/albumcover/changed.jpg")
                }
                "cover" if [14, 17].contains(&at) => {
                    value["sl_data"]["pic"] =
                        json!("https://img4.kuwo.cn/star/albumcover/changed.jpg");
                    value["sl_data"]["big_pic"] = value["sl_data"]["pic"].clone();
                }
                "tracks" if [15, 16].contains(&at) => value["info"]["musiclist"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 1),
                _ => {}
            }
            *row = json_response(&value);
        }
        let mut f = fixture::setup(rows).await;
        let e = f
            .client
            .native_submit_playlist(&credential(), "101", &request(None, false))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict, "{change}");
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(e.details["playlist_write_outcome"], "unconfirmed");
        assert_eq!(e.details["submission_outcome"], "not_dispatched");
        assert_eq!(e.details["automatic_retry"], false);
        assert!(
            fixture::requests(&mut f, 19)
                .await
                .iter()
                .all(|v| !v.contains("user_songlist_up"))
        );
    }
}
