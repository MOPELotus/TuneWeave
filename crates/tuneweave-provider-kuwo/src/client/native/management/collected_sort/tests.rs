use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::collections::tests::{item, pages},
        tests as fixture,
    },
};
use serde_json::Value;

fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
fn ordered_ids(reverse: bool) -> Vec<u64> {
    let mut ids: Vec<_> = (101..122).collect();
    if reverse {
        ids.reverse();
    } else {
        ids.rotate_left(1);
    }
    ids
}
fn order(ids: &[u64], account: Option<&str>) -> PlaylistOrderRequest {
    PlaylistOrderRequest {
        playlist_refs: ids
            .iter()
            .map(|id| ResourceRef::new(Platform::Kuwo, id.to_string()).unwrap())
            .collect(),
        account: account.map(str::to_owned),
    }
}
pub(crate) fn request(account: Option<&str>, reverse: bool) -> PlaylistOrderRequest {
    order(&ordered_ids(reverse), account)
}
fn change(uid: &str, before: &[Value], after: &[Value]) -> Vec<Vec<u8>> {
    let mut replies = vec![json_response(&json!({"result":"ok"}))];
    replies.extend(pages(uid, before));
    replies.push(json_response(&json!({"errcode":0,"result":"ok","uid":uid})));
    replies.extend(pages(uid, after));
    replies
}
pub(crate) fn flow(uid: &str, reverse: bool) -> Vec<Vec<u8>> {
    change(
        uid,
        &(101..122).map(item).collect::<Vec<_>>(),
        &ordered_ids(reverse)
            .into_iter()
            .map(item)
            .collect::<Vec<_>>(),
    )
}
fn assert_unconfirmed(e: &TuneWeaveError) {
    assert_eq!(e.details["write_outcome"], "unconfirmed");
    assert_eq!(e.details["write_requests_dispatched"], 1);
    assert!(!e.retryable);
    assert!(!format!("{e:?}").contains("selected-session"));
}

#[tokio::test]
async fn native_collected_sort_sdk_sends_complete_raw_id_array_to_the_distinct_cloud_operation() {
    for reverse in [false, true] {
        let r = request(None, reverse);
        let mut f = fixture::setup(flow("42", reverse)).await;
        let result = f
            .client
            .native_reorder_collected_playlists(&credential(), &r)
            .await
            .unwrap();
        assert_eq!(result.playlist_refs, r.playlist_refs);
        assert_eq!(result.extensions["library_section"], "collected");
        assert_eq!(result.extensions["confirmed"], true);
        assert_eq!(result.extensions["changed"], true);
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(result.extensions["atomic"], false);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("selected-session")
        );
        let seen = fixture::requests(&mut f, 6).await;
        assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 1);
        assert!(
            seen[3].starts_with("POST /pl.svc?op=pl3_sortfavorlist&uid=42&sid=selected-session&")
        );
        assert!(!seen[3].contains("recommend="));
        assert!(seen[3].contains("loginUid=42,loginSid=selected-session,"));
        assert!(
            seen[3]
                .to_ascii_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        let payload: Value =
            serde_json::from_str(seen[3].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload, json!({"data":ordered_ids(reverse)}));
        for i in [1, 4] {
            assert!(seen[i].contains("count=20&start=0"));
        }
        for i in [2, 5] {
            assert!(seen[i].contains("count=20&start=20"));
        }
        assert!(seen.iter().all(|r| !r.contains("pl3_getuserlists")
            && !r.contains("pl3_sortlist")
            && !r.contains("together")
            && !r.contains("op=like&")));
    }
}

#[tokio::test]
async fn native_collected_sort_confirmed_noop_reads_all_pages_including_empty_tail() {
    for count in [1, 20, 21] {
        let ids: Vec<_> = (101..101 + count).rev().collect();
        let r = order(&ids, None);
        let mut replies = vec![json_response(&json!({"result":"ok"}))];
        replies.extend(pages("42", &ids.into_iter().map(item).collect::<Vec<_>>()));
        let count = replies.len();
        let mut f = fixture::setup(replies).await;
        let result = f
            .client
            .native_reorder_collected_playlists(&credential(), &r)
            .await
            .unwrap();
        assert_eq!(result.extensions["confirmed"], true);
        assert_eq!(result.extensions["changed"], false);
        assert_eq!(result.extensions["write_requests_dispatched"], 0);
        assert!(
            fixture::requests(&mut f, count)
                .await
                .iter()
                .all(|r| r.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn native_collected_sort_rejects_local_invalid_or_incomplete_permutations_before_write() {
    let f = fixture::setup(vec![]).await;
    for refs in [
        vec![],
        order(&[101, 101], None).playlist_refs,
        order(&(1..=2000).collect::<Vec<_>>(), None).playlist_refs,
        vec![ResourceRef::new(Platform::Kuwo, "0101").unwrap()],
        vec![ResourceRef::new(Platform::Soda, "101").unwrap()],
        vec![ResourceRef::new(Platform::Kuwo, "9223372036854775808").unwrap()],
    ] {
        let r = PlaylistOrderRequest {
            playlist_refs: refs,
            account: None,
        };
        assert_eq!(
            f.client
                .native_reorder_collected_playlists(&credential(), &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.client
            .native_reorder_collected_playlists(&credential(), &request(Some("personal"), true))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for ids in [
        (101..121).collect::<Vec<_>>(),
        (101..123).collect(),
        (201..222).collect(),
    ] {
        let mut replies = flow("42", true);
        replies.truncate(3);
        let mut f = fixture::setup(replies).await;
        let e = f
            .client
            .native_reorder_collected_playlists(&credential(), &order(&ids, None))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(e.details.get("write_outcome").is_none());
        assert!(
            fixture::requests(&mut f, 3)
                .await
                .iter()
                .all(|r| r.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn native_collected_sort_never_uses_default_zero_or_unrelated_ack_as_success() {
    let mut bad: Vec<_> = [
        json!({}),
        json!({"result":"ok"}),
        json!({"opret":"ok"}),
        json!({"errcode":603}),
        json!({"errcode":0,"uid":43}),
        json!({"errcode":0,"result":"fail"}),
        json!({"errcode":null}),
        json!({"errcode":true}),
    ]
    .iter()
    .map(json_response)
    .collect();
    bad.extend([
        response(
            200,
            "application/json",
            "",
            br#"{"errcode":0,"errcode":603}"#,
        ),
        response(200, "text/html", "", br#"{"errcode":0}"#),
        response(
            302,
            "application/json",
            "Location: https://example.invalid/\r\n",
            b"",
        ),
        response(503, "application/json", "", b"selected-session"),
        response(200, "application/json", "", &vec![b' '; ACK_LIMIT + 1]),
    ]);
    for reply in bad {
        let mut replies = flow("42", true);
        replies[3] = reply;
        replies.truncate(4);
        let mut f = fixture::setup(replies).await;
        assert_unconfirmed(
            &f.client
                .native_reorder_collected_playlists(&credential(), &request(None, true))
                .await
                .unwrap_err(),
        );
        assert_eq!(
            fixture::requests(&mut f, 4)
                .await
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn native_collected_sort_preflight_malformed_or_cross_page_duplicates_never_dispatch() {
    for boundary in [1, 2] {
        for bad in [
            json!({"result":"fail","data":[]}),
            json!({"result":"ok","uid":43,"data":[]}),
            json!({"result":"ok","data":[item(101),item(101)]}),
            json!({"result":"ok","data":[{"id":333,"name":"selected-session"}]}),
        ] {
            let mut replies = flow("42", true);
            replies[boundary] = json_response(&bad);
            replies.truncate(boundary + 1);
            let mut f = fixture::setup(replies).await;
            let e = f
                .client
                .native_reorder_collected_playlists(&credential(), &request(None, true))
                .await
                .unwrap_err();
            assert!(e.details.get("write_outcome").is_none());
            assert!(!format!("{e:?}").contains("selected-session"));
            assert!(
                fixture::requests(&mut f, boundary + 1)
                    .await
                    .iter()
                    .all(|r| r.starts_with("GET "))
            );
        }
    }
    let mut replies = flow("42", true);
    replies[2] = json_response(&json!({"result":"ok","data":[item(101)]}));
    replies.truncate(3);
    let mut f = fixture::setup(replies).await;
    assert!(
        f.client
            .native_reorder_collected_playlists(&credential(), &request(None, true))
            .await
            .is_err()
    );
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_collected_sort_readback_requires_order_membership_and_all_known_metadata() {
    for reverse in [false, true] {
        let expected: Vec<_> = ordered_ids(reverse).into_iter().map(item).collect();
        let mut variants = Vec::new();
        for (field, value) in [
            ("name", json!("changed")),
            ("desc", json!("changed")),
            ("total", json!(1)),
            ("pic", json!("https://img1.kuwo.cn/a.jpg")),
        ] {
            let mut changed = expected.clone();
            changed[0][field] = value;
            variants.push(changed);
        }
        let mut changed = expected.clone();
        changed.swap(0, 1);
        variants.push(changed);
        let mut changed = expected.clone();
        changed.remove(0);
        variants.push(changed);
        let mut changed = expected.clone();
        changed.push(item(888));
        variants.push(changed);
        let mut changed = expected.clone();
        changed.push(item(101));
        variants.push(changed);
        for changed in variants {
            let replies = change("42", &(101..122).map(item).collect::<Vec<_>>(), &changed);
            let count = replies.len();
            let mut f = fixture::setup(replies).await;
            assert_unconfirmed(
                &f.client
                    .native_reorder_collected_playlists(&credential(), &request(None, reverse))
                    .await
                    .unwrap_err(),
            );
            fixture::requests(&mut f, count).await;
        }
    }
}

#[tokio::test]
async fn native_collected_sort_maximum_complete_collection_preserves_same_names_and_max_i64_ids() {
    let ids: Vec<_> = (0..library::MAX_SAVED as u64)
        .map(|i| i64::MAX as u64 - i)
        .collect();
    let before: Vec<_> = ids.iter().copied().map(item).collect();
    let mut after = before.clone();
    after.reverse();
    let requested: Vec<_> = ids.into_iter().rev().collect();
    let replies = change("42", &before, &after);
    let count = replies.len();
    let mut f = fixture::setup(replies).await;
    let result = f
        .client
        .native_reorder_collected_playlists(&credential(), &order(&requested, None))
        .await
        .unwrap();
    assert_eq!(result.playlist_refs.len(), 1999);
    let seen = fixture::requests(&mut f, count).await;
    let post = seen.iter().find(|r| r.starts_with("POST ")).unwrap();
    let body: Value = serde_json::from_str(post.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body, json!({"data":requested}));
    assert_eq!(body["data"][1998].as_i64(), Some(i64::MAX));
}

#[tokio::test]
async fn native_collected_sort_unknown_tokens_and_cookies_never_change_or_escape_the_account() {
    let mut before: Vec<_> = (101..122).map(item).collect();
    let mut after: Vec<_> = ordered_ids(true).into_iter().map(item).collect();
    before[0]["token"] = json!("never-export-before");
    after[20]["token"] = json!("never-export-after");
    let mut replies = change("42", &before, &after);
    replies[3] = response(
        200,
        "application/json",
        "Set-Cookie: sid=never-export-cookie; Path=/\r\n",
        br#"{"errcode":0,"uid":42,"token":"never-export-ack"}"#,
    );
    let mut f = fixture::setup(replies).await;
    let result = f
        .client
        .native_reorder_collected_playlists(&credential(), &request(None, true))
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("never-export")
    );
    let seen = fixture::requests(&mut f, 6).await;
    assert!(seen[4].contains("selected-session"));
    assert!(!seen[4].contains("never-export"));
}
