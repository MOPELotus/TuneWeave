pub(crate) use super::super::items::tests::request;
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        playlist::{favorite_tests::directory, tests::page},
        tests as fixture,
    },
};
use serde_json::Value;

fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
pub(crate) fn snapshot(uid: &str, ids: &[u64]) -> Vec<Vec<u8>> {
    let mut d = directory(Some(ids.len() as u64));
    d["uid"] = json!(uid);
    let mut bodies = vec![json_response(&d)];
    let pages = ids.len().div_ceil(1000).max(1) as u64;
    for _ in 0..2 {
        if ids.is_empty() {
            bodies.push(json_response(&page(&[], 1)));
        } else {
            for chunk in ids.chunks(1000) {
                bodies.push(json_response(&page(chunk, pages)));
            }
        }
    }
    bodies.push(json_response(&d));
    bodies
}
fn custom_flow(uid: &str, before: &[u64], after: &[u64]) -> Vec<Vec<u8>> {
    let mut bodies = vec![json_response(&json!({"result":"ok"}))];
    bodies.extend(snapshot(uid, before));
    bodies.push(json_response(&json!({"errcode":0,"pid":901,"uid":uid})));
    bodies.extend(snapshot(uid, after));
    bodies
}
pub(crate) fn flow(uid: &str, action: PlaylistItemMutationAction) -> Vec<Vec<u8>> {
    custom_flow(
        uid,
        &[11, 22, 22, 33],
        if action == PlaylistItemMutationAction::Add {
            &[44, 11, 22, 22, 33]
        } else {
            &[11, 33]
        },
    )
}
fn edit(body: &mut Vec<u8>, f: impl FnOnce(&mut Value)) {
    let separator = body.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let mut v: Value = serde_json::from_slice(&body[separator + 4..]).unwrap();
    f(&mut v);
    *body = json_response(&v);
}
fn payload(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn native_favorite_writes_sdk_targets_actual_system_id_and_confirms_both_states() {
    for subscribed in [true, false] {
        let action = if subscribed {
            PlaylistItemMutationAction::Add
        } else {
            PlaylistItemMutationAction::Remove
        };
        let mut f = fixture::setup(flow("42", action)).await;
        let id = if subscribed { "44" } else { "22" };
        let r = f
            .client
            .native_set_track_subscription(&credential(), id, subscribed)
            .await
            .unwrap();
        assert_eq!(
            r.resource_ref,
            ResourceRef::new(Platform::Kuwo, id).unwrap()
        );
        assert_eq!(r.subscribed, subscribed);
        assert_eq!(r.extensions["favorite_playlist_ref"], "kuwo:901");
        assert_eq!(r.extensions["library_owner_id"], "42");
        assert_eq!(
            r.extensions["cloud_track_count"],
            if subscribed { 5 } else { 2 }
        );
        assert_eq!(r.extensions["confirmed"], true);
        assert_eq!(r.extensions["changed"], true);
        assert_eq!(r.extensions["atomic"], false);
        assert_eq!(r.extensions["write_requests_dispatched"], 1);
        assert_eq!(
            r.extensions["sent_occurrences"],
            if subscribed { 1 } else { 2 }
        );
        assert!(
            r.extensions["source_snapshot_id"]
                .as_str()
                .unwrap()
                .starts_with("kuwo-native_favorite_playlist-")
        );
        assert!(
            !serde_json::to_string(&r)
                .unwrap()
                .contains("selected-session")
        );
        let seen = fixture::requests(&mut f, 10).await;
        assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 1);
        assert!(seen[5].contains(if subscribed {
            "op=pl3_add&"
        } else {
            "op=pl3_delete&"
        }));
        assert_eq!(
            payload(&seen[5]),
            if subscribed {
                json!({"pid":901,"data":[44]})
            } else {
                json!({"pid":901,"data":[22,22]})
            }
        );
        assert!(seen[5].contains("loginUid=42,loginSid=selected-session,"));
        assert!(
            seen[5]
                .to_ascii_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        for r in seen.iter().skip(1) {
            assert!(r.contains("uid=42&") && r.contains("sid=selected-session&"));
            assert!(
                !r.contains("pl3_addlist")
                    && !r.contains("ucheck")
                    && !r.contains("pl3_sort")
                    && !r.contains("get_songlist_info2")
            );
            assert!(!r.to_ascii_lowercase().contains("\r\ncookie:"));
        }
    }
}

#[tokio::test]
async fn native_favorite_writes_noop_requires_complete_stable_read_and_sends_nothing() {
    for (id, subscribed) in [("22", true), ("44", false)] {
        let mut bodies = vec![json_response(&json!({"result":"ok"}))];
        bodies.extend(snapshot("42", &[11, 22, 22, 33]));
        let mut f = fixture::setup(bodies).await;
        let r = f
            .client
            .native_set_track_subscription(&credential(), id, subscribed)
            .await
            .unwrap();
        assert_eq!(r.subscribed, subscribed);
        assert_eq!(r.extensions["changed"], false);
        assert_eq!(r.extensions["write_requests_dispatched"], 0);
        assert_eq!(r.extensions["sent_occurrences"], 0);
        assert_eq!(r.extensions["cloud_track_count"], 4);
        assert!(
            fixture::requests(&mut f, 5)
                .await
                .iter()
                .all(|r| r.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn native_favorite_writes_reject_invalid_ids_and_absent_or_ambiguous_system_lists() {
    let f = fixture::setup(vec![]).await;
    for id in ["", "0", "022", "-1", "22&pid=101", "9223372036854775808"] {
        let e = f
            .client
            .native_set_track_subscription(&credential(), id, true)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
    }
    let mut cases = vec![json!({"errcode":0,"plist":[]})];
    for (k, v) in [
        ("id", json!(0)),
        ("id", json!("0901")),
        ("type", json!("GENERAL")),
        ("type", json!("PC_DEFAULT")),
    ] {
        let mut d = directory(Some(0));
        d["plist"][0][k] = v;
        d["plist"][0]["title"] = json!("我喜欢听");
        cases.push(d);
    }
    let mut d = directory(Some(0));
    let mut duplicate = d["plist"][0].clone();
    duplicate["id"] = json!(902);
    d["plist"].as_array_mut().unwrap().push(duplicate);
    cases.push(d);
    for subscribed in [true, false] {
        for d in &cases {
            let mut f = fixture::setup(vec![
                json_response(&json!({"result":"ok"})),
                json_response(d),
            ])
            .await;
            let e = f
                .client
                .native_set_track_subscription(&credential(), "22", subscribed)
                .await
                .unwrap_err();
            assert!(e.details.get("write_outcome").is_none());
            assert!(
                fixture::requests(&mut f, 2)
                    .await
                    .iter()
                    .all(|r| r.starts_with("GET "))
            );
        }
    }
}

#[tokio::test]
async fn native_favorite_writes_ack_and_order_failures_are_unconfirmed_without_retry() {
    let acks = vec![
        json_response(&json!({"errcode":603})),
        json_response(&json!({"errcode":0,"pid":902})),
        json_response(&json!({"errcode":0,"uid":43})),
        response(
            302,
            "application/json",
            "Location: https://evil.test/\r\n",
            b"",
        ),
        response(200, "text/html", "", b"not json"),
        response(200, "application/json", "", &vec![b'x'; 65537]),
    ];
    for subscribed in [true, false] {
        let action = if subscribed {
            PlaylistItemMutationAction::Add
        } else {
            PlaylistItemMutationAction::Remove
        };
        for ack in &acks {
            let mut bodies = flow("42", action);
            bodies.truncate(6);
            bodies[5] = ack.clone();
            let mut f = fixture::setup(bodies).await;
            let e = f
                .client
                .native_set_track_subscription(
                    &credential(),
                    if subscribed { "44" } else { "22" },
                    subscribed,
                )
                .await
                .unwrap_err();
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert_eq!(e.details["write_requests_dispatched"], 1);
            assert!(!e.retryable);
            fixture::requests(&mut f, 6).await;
        }
    }
    for (subscribed, after) in [
        (true, vec![44, 11, 22, 33]),
        (true, vec![44, 44, 11, 22, 22, 33]),
        (true, vec![44, 33, 22, 22, 11]),
        (false, vec![11, 22, 33]),
        (false, vec![33, 11]),
        (false, vec![]),
    ] {
        let mut f = fixture::setup(custom_flow("42", &[11, 22, 22, 33], &after)).await;
        let e = f
            .client
            .native_set_track_subscription(
                &credential(),
                if subscribed { "44" } else { "22" },
                subscribed,
            )
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        fixture::requests(&mut f, 10).await;
    }
}

#[tokio::test]
async fn native_favorite_writes_bind_identity_category_and_known_metadata_across_write() {
    for (key, value) in [
        ("id", json!(902)),
        ("type", json!("GENERAL")),
        ("ispub", json!(true)),
        ("info", json!("changed")),
        ("turn", json!(1)),
    ] {
        let mut bodies = flow("42", PlaylistItemMutationAction::Add);
        for i in [6, 9] {
            edit(&mut bodies[i], |d| d["plist"][0][key] = value.clone());
        }
        let expected = if key == "id" || key == "type" { 7 } else { 10 };
        bodies.truncate(expected);
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_set_track_subscription(&credential(), "44", true)
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        fixture::requests(&mut f, expected).await;
    }
    let mut bodies = flow("42", PlaylistItemMutationAction::Add);
    for i in [6, 9] {
        edit(&mut bodies[i], |d| {
            d["plist"][0]["pic"] = json!("https://img4.kuwo.cn/star/albumcover/new.jpg")
        });
    }
    let mut f = fixture::setup(bodies).await;
    let r = f
        .client
        .native_set_track_subscription(&credential(), "44", true)
        .await
        .unwrap();
    assert_eq!(r.extensions["cover_changed"], true);
    fixture::requests(&mut f, 10).await;
}

#[tokio::test]
async fn native_favorite_writes_empty_and_multi_page_duplicate_deletion_are_complete() {
    for (before, after, subscribed, id) in [
        (vec![], vec![44], true, "44"),
        (vec![22], vec![], false, "22"),
        (vec![22; 1001], vec![], false, "22"),
    ] {
        let before_pages = before.len().div_ceil(1000).max(1);
        let count = before.len();
        let mut f = fixture::setup(custom_flow("42", &before, &after)).await;
        let r = f
            .client
            .native_set_track_subscription(&credential(), id, subscribed)
            .await
            .unwrap();
        assert_eq!(r.extensions["cloud_track_count"], after.len());
        assert_eq!(
            r.extensions["sent_occurrences"],
            if subscribed { 1 } else { count }
        );
        let seen = fixture::requests(&mut f, 8 + before_pages * 2).await;
        assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 1);
        if count == 1001 {
            assert_eq!(
                payload(&seen[3 + before_pages * 2])["data"],
                json!(vec![22; 1001])
            );
        }
    }
}
