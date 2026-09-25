use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
pub(crate) fn item(id: u64) -> Value {
    json!({"id":id,"name":"同名收藏 + %20","desc":"原始简介\n第二行","total":0})
}
fn items(present: bool) -> Vec<Value> {
    let mut values: Vec<_> = (101..122).map(item).collect();
    if present {
        values.insert(10, item(999));
    }
    values
}
pub(crate) fn pages(uid: &str, values: &[Value]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for chunk in values.chunks(20) {
        out.push(json_response(
            &json!({"result":"ok","uid":uid,"data":chunk}),
        ));
    }
    if values.len() % 20 == 0 {
        out.push(json_response(&json!({"result":"ok","uid":uid,"data":[]})));
    }
    out
}
fn change(uid: &str, before: &[Value], after: &[Value], opret: &str) -> Vec<Vec<u8>> {
    let mut out = vec![json_response(&json!({"result":"ok"}))];
    out.extend(pages(uid, before));
    out.push(json_response(&json!({"opret":opret})));
    out.extend(pages(uid, after));
    out
}
pub(crate) fn flow(uid: &str, subscribed: bool) -> Vec<Vec<u8>> {
    change(uid, &items(!subscribed), &items(subscribed), "ok")
}
fn is_write(request: &str) -> bool {
    request.lines().next().unwrap().contains("op=like&")
}
fn assert_unconfirmed(error: &TuneWeaveError) {
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(error.details["write_requests_dispatched"], 1);
    assert!(!error.retryable);
    assert!(!format!("{error:?}").contains("selected-session"));
}

#[tokio::test]
async fn native_collection_sdk_uses_one_business_get_and_directional_ack_with_complete_pages() {
    for subscribed in [false, true] {
        for opret in [
            "ok",
            if subscribed {
                "collected"
            } else {
                "notcollected"
            },
        ] {
            let mut f =
                fixture::setup(change("42", &items(!subscribed), &items(subscribed), opret)).await;
            let value = f
                .client
                .native_set_playlist_subscription(&credential(), "999", subscribed)
                .await
                .unwrap();
            assert_eq!(value.resource_ref.to_string(), "kuwo:999");
            assert_eq!(value.subscribed, subscribed);
            assert_eq!(value.extensions["confirmed"], true);
            assert_eq!(value.extensions["changed"], true);
            assert_eq!(value.extensions["write_requests_dispatched"], 1);
            assert_eq!(value.extensions["library_section"], "collected");
            assert_eq!(value.extensions["library_owner_id"], "42");
            assert_eq!(value.extensions["atomic"], false);
            assert!(
                !serde_json::to_string(&value)
                    .unwrap()
                    .contains("selected-session")
            );
            let seen = fixture::requests(&mut f, 6).await;
            assert!(seen.iter().all(|r| r.starts_with("GET ")));
            assert_eq!(seen.iter().filter(|r| is_write(r)).count(), 1);
            assert!(seen[3].starts_with(&format!("GET /pl.svc?op=like&type=PLAYLIST&bigid=1&act={}&uid=42&sid=selected-session&sourceid=999 HTTP/1.1", if subscribed { "add" } else { "delete" })));
            assert!(seen[3].contains("loginUid=42,loginSid=selected-session,"));
            assert_eq!(seen[3].split_once("\r\n\r\n").unwrap().1, "");
            for i in [1, 4] {
                assert!(seen[i].contains("count=20&start=0"));
            }
            for i in [2, 5] {
                assert!(seen[i].contains("count=20&start=20"));
            }
            assert!(seen.iter().all(|r| !r.contains("pl3_")
                && !r.contains("together")
                && !r.contains("likemultipl")
                && !r.contains("songlist?")));
        }
    }
}

#[tokio::test]
async fn native_collection_noop_requires_complete_directory_and_handles_empty_or_full_last_page() {
    for subscribed in [false, true] {
        for count in [0, 20, 21] {
            let mut values: Vec<_> = (101..101 + count).map(item).collect();
            if subscribed {
                values.push(item(999));
            }
            let mut replies = vec![json_response(&json!({"result":"ok"}))];
            replies.extend(pages("42", &values));
            let requests = replies.len();
            let mut f = fixture::setup(replies).await;
            let result = f
                .client
                .native_set_playlist_subscription(&credential(), "999", subscribed)
                .await
                .unwrap();
            assert_eq!(result.extensions["changed"], false);
            assert_eq!(result.extensions["write_requests_dispatched"], 0);
            assert!(
                fixture::requests(&mut f, requests)
                    .await
                    .iter()
                    .all(|r| !is_write(r))
            );
        }
    }
}

#[tokio::test]
async fn native_collection_invalid_local_ids_never_authenticate_or_write() {
    let f = fixture::setup(vec![]).await;
    for id in [
        "",
        "0",
        "0999",
        "+999",
        "-1",
        "999 ",
        "999&act=delete",
        "9223372036854775808",
        "kuwo:999",
    ] {
        for subscribed in [false, true] {
            assert_eq!(
                f.client
                    .native_set_playlist_subscription(&credential(), id, subscribed)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }
    assert!(f.seen.is_empty());
}

#[tokio::test]
async fn native_collection_ack_rejections_never_read_back_retry_or_expose_response() {
    for subscribed in [false, true] {
        let mut bad: Vec<Vec<u8>> = [
            json!({"errcode":0}),
            json!({"opret":null}),
            json!({"opret":true}),
            json!({"opret": if subscribed { "notcollected" } else { "collected" }}),
            json!({"opret":"OK"}),
            json!({"opret":"selected-session"}),
            json!({"opret":"ok","uid":43}),
            json!({"opret":"ok","sourceid":998}),
            json!({"opret":"ok","pid":998}),
            json!({"opret":"ok","errcode":603}),
            json!({"opret":"ok","result":"fail"}),
        ]
        .iter()
        .map(json_response)
        .collect();
        bad.extend([
            response(
                200,
                "application/json",
                "",
                br#"{"opret":"ok","opret":"fail"}"#,
            ),
            response(
                302,
                "application/json",
                "Location: https://example.invalid/\r\n",
                b"",
            ),
            response(200, "text/html", "", br#"{"opret":"ok"}"#),
            response(200, "application/json", "", &vec![b' '; ACK_LIMIT + 1]),
            response(503, "application/json", "", b"selected-session"),
        ]);
        for reply in bad {
            let mut replies = flow("42", subscribed);
            replies[3] = reply;
            replies.truncate(4);
            let mut f = fixture::setup(replies).await;
            let error = f
                .client
                .native_set_playlist_subscription(&credential(), "999", subscribed)
                .await
                .unwrap_err();
            assert_unconfirmed(&error);
            assert_eq!(
                fixture::requests(&mut f, 4)
                    .await
                    .iter()
                    .filter(|r| is_write(r))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn native_collection_incomplete_changed_or_malformed_preflight_never_writes() {
    for subscribed in [false, true] {
        for boundary in [1, 2] {
            for bad in [
                json!({"result":"fail","data":[]}),
                json!({"result":"ok","uid":43,"data":[]}),
                json!({"result":"ok","data":[item(101),item(101)]}),
                json!({"result":"ok","data":[{"id":333,"name":"selected-session"}]}),
            ] {
                let mut replies = flow("42", subscribed);
                replies[boundary] = json_response(&bad);
                replies.truncate(boundary + 1);
                let mut f = fixture::setup(replies).await;
                let e = f
                    .client
                    .native_set_playlist_subscription(&credential(), "999", subscribed)
                    .await
                    .unwrap_err();
                assert!(e.details.get("write_outcome").is_none());
                assert!(!format!("{e:?}").contains("selected-session"));
                assert!(
                    fixture::requests(&mut f, boundary + 1)
                        .await
                        .iter()
                        .all(|r| !is_write(r))
                );
            }
        }
        // A duplicate occurring on a later page cannot be mistaken for completion.
        let mut replies = flow("42", subscribed);
        replies[2] = json_response(&json!({"result":"ok","data":[item(101)]}));
        replies.truncate(3);
        let mut f = fixture::setup(replies).await;
        assert!(
            f.client
                .native_set_playlist_subscription(&credential(), "999", subscribed)
                .await
                .is_err()
        );
        assert!(
            fixture::requests(&mut f, 3)
                .await
                .iter()
                .all(|r| !is_write(r))
        );
    }
}

#[tokio::test]
async fn native_collection_readback_preserves_members_known_metadata_and_relative_order() {
    for subscribed in [false, true] {
        let expected = items(subscribed);
        let mut variants = Vec::new();
        for (field, value) in [
            ("name", json!("changed")),
            ("desc", json!("changed")),
            ("total", json!(1)),
            ("pic", json!("https://img1.kuwo.cn/a.jpg")),
        ] {
            let mut altered = expected.clone();
            altered[0][field] = value;
            variants.push(altered);
        }
        let mut altered = expected.clone();
        altered.swap(0, 1);
        variants.push(altered);
        let mut altered = expected.clone();
        altered.remove(0);
        variants.push(altered);
        let mut altered = expected.clone();
        altered.push(item(888));
        variants.push(altered);
        variants.push(items(!subscribed));
        let mut altered = expected.clone();
        altered.push(item(101));
        variants.push(altered);
        for altered in variants {
            let replies = change("42", &items(!subscribed), &altered, "ok");
            let count = replies.len();
            let mut f = fixture::setup(replies).await;
            let e = f
                .client
                .native_set_playlist_subscription(&credential(), "999", subscribed)
                .await
                .unwrap_err();
            assert_unconfirmed(&e);
            assert_eq!(
                fixture::requests(&mut f, count)
                    .await
                    .iter()
                    .filter(|r| is_write(r))
                    .count(),
                1
            );
        }
        // A late page failing after the GET is an unconfirmed write, never partial success.
        let mut replies = flow("42", subscribed);
        replies[5] = response(503, "application/json", "", b"selected-session");
        let mut f = fixture::setup(replies).await;
        assert_unconfirmed(
            &f.client
                .native_set_playlist_subscription(&credential(), "999", subscribed)
                .await
                .unwrap_err(),
        );
        fixture::requests(&mut f, 6).await;
    }
}

#[tokio::test]
async fn native_collection_limit_preflight_and_maximum_supported_directory_are_complete() {
    let full: Vec<_> = (10000..10000 + library::MAX_SAVED as u64)
        .map(item)
        .collect();
    let mut replies = vec![json_response(&json!({"result":"ok"}))];
    replies.extend(pages("42", &full));
    let mut f = fixture::setup(replies).await;
    let e = f
        .client
        .native_set_playlist_subscription(&credential(), "999", true)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    assert!(
        fixture::requests(&mut f, 101)
            .await
            .iter()
            .all(|r| !is_write(r))
    );

    // 100 full pages are incomplete, even when the requested target appears early.
    let mut over = full.clone();
    over.push(item(999));
    let mut replies = vec![json_response(&json!({"result":"ok"}))];
    replies.extend(pages("42", &over).into_iter().take(100));
    let mut f = fixture::setup(replies).await;
    assert!(
        f.client
            .native_set_playlist_subscription(&credential(), "10000", true)
            .await
            .is_err()
    );
    assert!(
        fixture::requests(&mut f, 101)
            .await
            .iter()
            .all(|r| !is_write(r))
    );

    // Removing from 1999, and adding back to 1999, each confirms the last short page.
    let mut smaller = full.clone();
    smaller.remove(0);
    for subscribed in [false, true] {
        let (before, after) = if subscribed {
            (&smaller, &full)
        } else {
            (&full, &smaller)
        };
        let replies = change("42", before, after, "ok");
        let count = replies.len();
        let mut f = fixture::setup(replies).await;
        let result = f
            .client
            .native_set_playlist_subscription(&credential(), "10000", subscribed)
            .await
            .unwrap();
        assert_eq!(result.subscribed, subscribed);
        assert_eq!(
            fixture::requests(&mut f, count)
                .await
                .iter()
                .filter(|r| is_write(r))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn native_collection_maximum_id_and_unknown_tokens_do_not_change_or_export_membership() {
    for subscribed in [false, true] {
        let mut original = vec![item(101)];
        original[0]["token"] = json!("never-export-before");
        let mut present = original.clone();
        present.push(item(i64::MAX as u64));
        present[0]["token"] = json!("never-export-after");
        let (before, after) = if subscribed {
            (&original, &present)
        } else {
            (&present, &original)
        };
        let mut replies = change("42", before, after, "ok");
        replies[2]=response(200,"application/json","Set-Cookie: sid=never-export-cookie; Path=/\r\n",format!("{{\"opret\":\"ok\",\"uid\":42,\"pid\":{},\"sourceid\":{},\"token\":\"never-export-ack\"}}",i64::MAX,i64::MAX).as_bytes());
        let mut f = fixture::setup(replies).await;
        let result = f
            .client
            .native_set_playlist_subscription(&credential(), &i64::MAX.to_string(), subscribed)
            .await
            .unwrap();
        assert_eq!(result.subscribed, subscribed);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("never-export")
        );
        let seen = fixture::requests(&mut f, 4).await;
        assert!(seen[2].contains("sourceid=9223372036854775807"));
        assert!(seen[3].contains("selected-session"));
        assert!(!seen[3].contains("never-export"));
    }
}
