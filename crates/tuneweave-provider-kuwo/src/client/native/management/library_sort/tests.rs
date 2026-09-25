use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

pub(crate) fn request(account: Option<&str>, reverse: bool) -> PlaylistOrderRequest {
    PlaylistOrderRequest {
        playlist_refs: refs(if reverse { &[102, 101] } else { &[101, 102] }),
        account: account.map(str::to_owned),
    }
}
fn refs(ids: &[u64]) -> Vec<ResourceRef> {
    ids.iter()
        .map(|id| ResourceRef::new(Platform::Kuwo, id.to_string()).unwrap())
        .collect()
}
fn general(id: u64, turn: Option<i32>) -> Value {
    let mut v = json!({"id":id,"type":"GENERAL","title":"同名歌单 + %20","info":"简介\n原文","musicnum":0,"ispub":false,"playnum":0,"recommend":0});
    if let Some(t) = turn {
        v["turn"] = json!(t);
    }
    v
}
fn directory(uid: &str, turns: [Option<i32>; 2]) -> Value {
    json!({"errcode":0,"result":"ok","uid":uid,"plist":[general(101,turns[0]),general(102,turns[1]),
        {"type":"MYFAVORITE","id":901,"title":"系统喜欢","musicnum":4,"ispub":false,"turn":-5},
        {"type":"MOBI_DEFAULT","hidden":1},{"type":"PC_DEFAULT","id":0,"hidden":"0"},
        {"type":"RADIO","id":700,"info":"系统信息","hidden":false},{"type":"ORDER","id":701}]})
}
fn after(uid: &str, reverse: bool) -> Value {
    directory(
        uid,
        if reverse {
            [Some(2), Some(1)]
        } else {
            [Some(1), Some(2)]
        },
    )
}
pub(crate) fn flow(uid: &str, reverse: bool) -> Vec<Vec<u8>> {
    [
        json!({"result":"ok"}),
        directory(uid, [Some(9), Some(-1)]),
        json!({"errcode":0,"uid":uid}),
        after(uid, reverse),
    ]
    .iter()
    .map(json_response)
    .collect()
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
fn payload(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn native_library_sort_sdk_sends_pid_position_object_and_verifies_explicit_turns() {
    for reverse in [false, true] {
        let r = request(None, reverse);
        let mut v = flow("42", reverse);
        let mut d = after("42", reverse);
        d["plist"].as_array_mut().unwrap().reverse();
        v[3] = json_response(&d);
        let mut f = fixture::setup(v).await;
        let value = f
            .client
            .native_reorder_account_playlists(&credential(), &r)
            .await
            .unwrap();
        assert_eq!(value.playlist_refs, r.playlist_refs);
        assert_eq!(value.extensions["confirmed"], true);
        assert_eq!(value.extensions["changed"], true);
        assert_eq!(value.extensions["write_requests_dispatched"], 1);
        assert_eq!(value.extensions["atomic"], false);
        assert_eq!(value.extensions["library_owner_id"], "42");
        let seen = fixture::requests(&mut f, 4).await;
        assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 1);
        assert!(seen[2].contains("op=pl3_sortlist&"));
        assert!(seen[2].contains("loginUid=42,loginSid=selected-session,"));
        assert!(
            seen[2]
                .to_ascii_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        assert_eq!(
            payload(&seen[2]),
            if reverse {
                json!({"101":2,"102":1})
            } else {
                json!({"101":1,"102":2})
            }
        );
        assert!(seen.iter().all(|r| !r.contains("ucheck")
            && !r.contains("get_songlist_info2")
            && !r.contains("pl3_add")
            && !r.contains("pl3_delete")
            && !r.contains("together")));
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("selected-session")
        );
    }
}

#[tokio::test]
async fn native_library_sort_noop_requires_canonical_positions_even_when_array_order_differs() {
    for reverse in [false, true] {
        let r = request(None, reverse);
        let mut f = fixture::setup(vec![
            json_response(&json!({"result":"ok"})),
            json_response(&after("42", reverse)),
        ])
        .await;
        let value = f
            .client
            .native_reorder_account_playlists(&credential(), &r)
            .await
            .unwrap();
        assert_eq!(value.extensions["changed"], false);
        assert_eq!(value.extensions["write_requests_dispatched"], 0);
        fixture::requests(&mut f, 2).await;
    }
    let mut r = request(None, false);
    r.playlist_refs.truncate(1);
    let d = json!({"errcode":0,"plist":[general(101,Some(1))]});
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&d),
    ])
    .await;
    assert_eq!(
        f.client
            .native_reorder_account_playlists(&credential(), &r)
            .await
            .unwrap()
            .extensions["changed"],
        false
    );
    fixture::requests(&mut f, 2).await;
}

#[tokio::test]
async fn native_library_sort_unknown_tied_zero_and_negative_old_positions_are_normalized() {
    for turns in [
        [None, None],
        [Some(1), None],
        [Some(1), Some(1)],
        [Some(0), Some(0)],
        [Some(-4), Some(-2)],
    ] {
        let mut v = flow("42", false);
        v[1] = json_response(&directory("42", turns));
        let mut f = fixture::setup(v).await;
        let value = f
            .client
            .native_reorder_account_playlists(&credential(), &request(None, false))
            .await
            .unwrap();
        assert_eq!(value.extensions["changed"], true);
        fixture::requests(&mut f, 4).await;
    }
}

#[tokio::test]
async fn native_library_sort_invalid_requests_and_incomplete_or_system_permutations_never_write() {
    let f = fixture::setup(vec![]).await;
    for ids in [
        vec![],
        refs(&[101, 101]),
        vec![ResourceRef::new(Platform::Soda, "101").unwrap()],
        vec![ResourceRef::new(Platform::Kuwo, "0101").unwrap()],
        refs(&(1..=4097).collect::<Vec<_>>()),
    ] {
        let mut r = request(None, false);
        r.playlist_refs = ids;
        assert_eq!(
            f.client
                .native_reorder_account_playlists(&credential(), &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.client
            .native_reorder_account_playlists(&credential(), &request(Some("personal"), false))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for ids in [
        vec![101],
        vec![101, 102, 999],
        vec![101, 901],
        vec![101, 700],
        vec![101, 999],
    ] {
        let mut r = request(None, false);
        r.playlist_refs = refs(&ids);
        let mut f = fixture::setup(flow("42", false)[..2].to_vec()).await;
        let e = f
            .client
            .native_reorder_account_playlists(&credential(), &r)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(e.details.get("write_outcome").is_none());
        fixture::requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn native_library_sort_ack_and_unproven_post_positions_are_unconfirmed_without_retry() {
    for ack in [
        json_response(&json!({"errcode":603})),
        json_response(&json!({"result":"ok"})),
        json_response(&json!({"errcode":0,"uid":43})),
        response(
            302,
            "application/json",
            "Location: https://evil.test/\r\n",
            b"",
        ),
        response(200, "text/html", "", b"html"),
        response(200, "application/json", "", &vec![b'x'; 65537]),
    ] {
        let mut v = flow("42", true);
        v.truncate(3);
        v[2] = ack;
        let mut f = fixture::setup(v).await;
        let e = f
            .client
            .native_reorder_account_playlists(&credential(), &request(None, true))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert!(!e.retryable);
        fixture::requests(&mut f, 3).await;
    }
    for turns in [
        [None, None],
        [Some(2), None],
        [Some(1), Some(1)],
        [Some(1), Some(2)],
        [Some(-1), Some(0)],
    ] {
        let mut v = flow("42", true);
        let mut d = directory("42", turns);
        d["plist"].as_array_mut().unwrap().swap(0, 1);
        v[3] = json_response(&d);
        let mut f = fixture::setup(v).await;
        let e = f
            .client
            .native_reorder_account_playlists(&credential(), &request(None, true))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        fixture::requests(&mut f, 4).await;
    }
}

#[tokio::test]
async fn native_library_sort_preserves_known_metadata_in_each_target_and_system_row() {
    for row in 0..7 {
        for (key, value) in [
            ("title", json!("changed")),
            ("info", json!("changed info")),
            ("pic", json!("https://img4.kuwo.cn/star/userpl/new.jpg")),
            ("ispub", json!(true)),
            ("musicnum", json!(8)),
            ("playnum", json!(9)),
            ("recommend", json!(1)),
            ("hidden", json!(2)),
        ] {
            let mut d = after("42", true);
            d["plist"][row][key] = value;
            let mut v = flow("42", true);
            v[3] = json_response(&d);
            let mut f = fixture::setup(v).await;
            let e = f
                .client
                .native_reorder_account_playlists(&credential(), &request(None, true))
                .await
                .unwrap_err();
            assert_eq!(
                e.details["write_outcome"], "unconfirmed",
                "row={row}, field={key}"
            );
            assert!(!e.retryable);
            fixture::requests(&mut f, 4).await;
        }
    }
}

#[tokio::test]
async fn native_library_sort_cannot_lose_change_or_duplicate_excluded_system_rows() {
    let mut cases = Vec::new();
    let mut d = after("42", true);
    d["plist"][2]["id"] = json!(902);
    cases.push(d);
    let mut d = after("42", true);
    d["plist"][2]["turn"] = json!(99);
    cases.push(d);
    let mut d = after("42", true);
    d["plist"][3]["type"] = json!("PC_DEFAULT");
    cases.push(d);
    let mut d = after("42", true);
    d["plist"].as_array_mut().unwrap().remove(3);
    cases.push(d);
    let mut d = after("42", true);
    let extra = d["plist"][3].clone();
    d["plist"].as_array_mut().unwrap().push(extra);
    cases.push(d);
    let mut d = after("42", true);
    d["plist"][0]["type"] = json!("RADIO");
    cases.push(d);
    let mut d = after("42", true);
    d["plist"][0]["id"] = json!(999);
    cases.push(d);
    for d in cases {
        let mut v = flow("42", true);
        v[3] = json_response(&d);
        let mut f = fixture::setup(v).await;
        let e = f
            .client
            .native_reorder_account_playlists(&credential(), &request(None, true))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        fixture::requests(&mut f, 4).await;
    }
}

#[tokio::test]
async fn native_library_sort_raw_system_fields_are_bounded_without_exporting_unknown_secrets() {
    for (field, value) in [
        ("title", json!("selected-session")),
        ("info", json!("x".repeat(16385))),
        ("pic", json!("x".repeat(2049))),
        ("hidden", json!({"untrusted":"object"})),
        ("hidden", json!("x".repeat(33))),
        ("recommend", json!(true)),
    ] {
        let mut d = directory("42", [None, None]);
        d["plist"][3][field] = value;
        let mut f = fixture::setup(vec![
            json_response(&json!({"result":"ok"})),
            json_response(&d),
        ])
        .await;
        let e = f
            .client
            .native_reorder_account_playlists(&credential(), &request(None, true))
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none());
        assert!(!format!("{e:?}").contains("selected-session"));
        fixture::requests(&mut f, 2).await;
    }
    let mut a = directory("42", [None, None]);
    let mut b = after("42", true);
    for (i, d) in [&mut a, &mut b].into_iter().enumerate() {
        for row in d["plist"].as_array_mut().unwrap() {
            row["token"] = json!(format!("selected-session-{i}"));
        }
    }
    let mut f = fixture::setup(
        [json!({"result":"ok"}), a, json!({"errcode":0}), b]
            .iter()
            .map(json_response)
            .collect(),
    )
    .await;
    let value = f
        .client
        .native_reorder_account_playlists(&credential(), &request(None, true))
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&value)
            .unwrap()
            .contains("selected-session")
    );
    fixture::requests(&mut f, 4).await;
}

#[tokio::test]
async fn native_library_sort_handles_4096_same_named_playlists_with_maximum_integer_ids() {
    let ids = ((i64::MAX as u64 - 4095)..=i64::MAX as u64).collect::<Vec<_>>();
    let reversed = ids.iter().copied().rev().collect::<Vec<_>>();
    let before =
        json!({"errcode":0,"plist":ids.iter().map(|id|general(*id,None)).collect::<Vec<_>>()});
    let after = json!({"errcode":0,"plist":ids.iter().enumerate().map(|(i,id)|general(*id,Some(4096-i as i32))).collect::<Vec<_>>()});
    let mut f = fixture::setup(
        [json!({"result":"ok"}), before, json!({"errcode":0}), after]
            .iter()
            .map(json_response)
            .collect(),
    )
    .await;
    let r = PlaylistOrderRequest {
        playlist_refs: refs(&reversed),
        account: None,
    };
    let value = f
        .client
        .native_reorder_account_playlists(&credential(), &r)
        .await
        .unwrap();
    assert_eq!(value.playlist_refs, r.playlist_refs);
    let seen = fixture::requests(&mut f, 4).await;
    let data = payload(&seen[2]);
    assert_eq!(data.as_object().unwrap().len(), 4096);
    assert_eq!(data[ids[0].to_string()], 4096);
    assert_eq!(data[ids[4095].to_string()], 1);
}
