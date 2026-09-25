use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{playlist::tests::directory, tests as fixture},
};
use serde_json::Value;

pub(crate) fn case(favorite: bool) -> (&'static str, usize, usize) {
    if favorite {
        ("901", 10, 5)
    } else {
        ("101", 14, 7)
    }
}
pub(crate) fn request(account: Option<&str>) -> PlaylistTrackOrderRequest {
    PlaylistTrackOrderRequest {
        track_refs: refs(&[22, 33, 11, 22]),
        account: account.map(str::to_owned),
    }
}
fn refs(ids: &[u64]) -> Vec<ResourceRef> {
    ids.iter()
        .map(|id| ResourceRef::new(Platform::Kuwo, id.to_string()).unwrap())
        .collect()
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
fn snapshot(uid: &str, favorite: bool, ids: &[u64]) -> Vec<Vec<u8>> {
    if favorite {
        super::super::favorites::tests::snapshot(uid, ids)
    } else {
        let mut v = super::super::items::tests::snapshot(uid, ids);
        let last = v.len() - 1;
        for i in [0, last] {
            edit(&mut v[i], |d| d["uid"] = json!(uid));
        }
        v
    }
}
fn custom_flow(uid: &str, favorite: bool, before: &[u64], after: &[u64]) -> Vec<Vec<u8>> {
    let mut v = vec![json_response(&json!({"result":"ok"}))];
    v.extend(snapshot(uid, favorite, before));
    v.push(json_response(
        &json!({"errcode":0,"uid":uid,"pid":case(favorite).0}),
    ));
    v.extend(snapshot(uid, favorite, after));
    v
}
pub(crate) fn flow(uid: &str, favorite: bool) -> Vec<Vec<u8>> {
    custom_flow(uid, favorite, &[11, 22, 22, 33], &[22, 33, 11, 22])
}
fn edit(body: &mut Vec<u8>, change: impl FnOnce(&mut Value)) {
    let pos = body.windows(4).position(|s| s == b"\r\n\r\n").unwrap();
    let mut v: Value = serde_json::from_slice(&body[pos + 4..]).unwrap();
    change(&mut v);
    *body = json_response(&v);
}
fn payload(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn native_track_sort_sdk_confirms_complete_duplicate_order_in_both_owned_categories() {
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        let r = request(None);
        let mut f = fixture::setup(flow("42", favorite)).await;
        let value = f
            .client
            .native_reorder_playlist_tracks(&credential(), id, &r)
            .await
            .unwrap();
        assert_eq!(value.playlist_ref.id(), id);
        assert_eq!(value.track_refs, r.track_refs);
        assert_eq!(value.extensions["confirmed"], true);
        assert_eq!(value.extensions["changed"], true);
        assert_eq!(value.extensions["atomic"], false);
        assert_eq!(value.extensions["write_requests_dispatched"], 1);
        assert_eq!(value.extensions["cloud_track_count"], 4);
        assert_eq!(
            value.extensions["library_section"],
            if favorite { "favorite" } else { "created" }
        );
        assert!(value.snapshot_id.as_deref().unwrap().contains(if favorite {
            "native_favorite_playlist"
        } else {
            "native_created_playlist"
        }));
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("selected-session")
        );
        let seen = fixture::requests(&mut f, boundaries).await;
        assert_eq!(seen.iter().filter(|s| s.starts_with("POST ")).count(), 1);
        assert!(seen[write_at].contains("op=pl3_sort&"));
        assert!(seen[write_at].contains("loginUid=42,loginSid=selected-session,"));
        assert!(
            seen[write_at]
                .to_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        assert_eq!(
            payload(&seen[write_at]),
            json!({"pid":id.parse::<u64>().unwrap(),"data":[22,33,11,22]})
        );
        assert!(seen.iter().all(|s| !s.contains("pl3_sortlist")
            && !s.contains("pl3_add")
            && !s.contains("pl3_delete")
            && !s.contains("ucheck")));
    }
}

#[tokio::test]
async fn native_track_sort_same_order_and_single_track_confirm_noop_without_post() {
    for favorite in [false, true] {
        for ids in [vec![11, 22, 22, 33], vec![22]] {
            let (id, _, write_at) = case(favorite);
            let mut v = vec![json_response(&json!({"result":"ok"}))];
            v.extend(snapshot("42", favorite, &ids));
            let mut f = fixture::setup(v).await;
            let mut r = request(None);
            r.track_refs = refs(&ids);
            let result = f
                .client
                .native_reorder_playlist_tracks(&credential(), id, &r)
                .await
                .unwrap();
            assert_eq!(result.track_refs, r.track_refs);
            assert_eq!(result.extensions["changed"], false);
            assert_eq!(result.extensions["write_requests_dispatched"], 0);
            assert!(
                fixture::requests(&mut f, write_at)
                    .await
                    .iter()
                    .all(|r| r.starts_with("GET "))
            );
        }
    }
}

#[tokio::test]
async fn native_track_sort_local_input_validation_and_full_permutation_preflight_send_no_writes() {
    let f = fixture::setup(vec![]).await;
    for id in ["0", "0101", "-1", "101?pid=901", "9223372036854775808"] {
        assert_eq!(
            f.client
                .native_reorder_playlist_tracks(&credential(), id, &request(None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for input in [
        vec![],
        vec![ResourceRef::new(Platform::Kuwo, "0").unwrap()],
        vec![ResourceRef::new(Platform::Soda, "22").unwrap()],
        refs(&vec![22; 10001]),
    ] {
        let mut r = request(None);
        r.track_refs = input;
        assert_eq!(
            f.client
                .native_reorder_playlist_tracks(&credential(), "101", &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.client
            .native_reorder_playlist_tracks(&credential(), "101", &request(Some("personal")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for favorite in [false, true] {
        for ids in [
            vec![11, 22, 33],
            vec![11, 22, 22, 33, 44],
            vec![11, 11, 22, 33],
            vec![11, 22, 22, 44],
        ] {
            let (id, _, write_at) = case(favorite);
            let mut v = flow("42", favorite);
            v.truncate(write_at);
            let mut f = fixture::setup(v).await;
            let mut r = request(None);
            r.track_refs = refs(&ids);
            let e = f
                .client
                .native_reorder_playlist_tracks(&credential(), id, &r)
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidRequest);
            assert!(e.details.get("write_outcome").is_none());
            fixture::requests(&mut f, write_at).await;
        }
    }
}

#[tokio::test]
async fn native_track_sort_never_falls_back_to_saved_or_other_system_categories() {
    for kind in ["MOBI_DEFAULT", "PC_DEFAULT", "RADIO", "ORDER"] {
        let mut d = directory(Some(4));
        d["plist"][0]["type"] = json!(kind);
        let mut f = fixture::setup(vec![
            json_response(&json!({"result":"ok"})),
            json_response(&d),
        ])
        .await;
        let e = f
            .client
            .native_reorder_playlist_tracks(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::PermissionDenied);
        fixture::requests(&mut f, 2).await;
    }
    for favorite in [false, true] {
        let (id, _, write_at) = case(favorite);
        let mut v = flow("42", favorite);
        edit(&mut v[write_at + 1], |d| {
            d["plist"][0]["type"] = json!(if favorite { "GENERAL" } else { "MYFAVORITE" });
            d["plist"][0]["title"] = json!("same id");
        });
        v.truncate(write_at + 2);
        let mut f = fixture::setup(v).await;
        let e = f
            .client
            .native_reorder_playlist_tracks(&credential(), id, &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        fixture::requests(&mut f, write_at + 2).await;
    }
}

#[tokio::test]
async fn native_track_sort_strict_ack_and_wrong_post_order_are_unconfirmed_without_retry() {
    let acks = [
        json_response(&json!({"errcode":603})),
        json_response(&json!({"errcode":0,"pid":99})),
        json_response(&json!({"errcode":0,"uid":43})),
        response(
            302,
            "application/json",
            "Location: https://evil.test/\r\n",
            b"",
        ),
        response(200, "text/html", "", b"html"),
        response(200, "application/json", "", &vec![b'x'; 65537]),
    ];
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        for ack in &acks {
            let mut v = flow("42", favorite);
            v.truncate(write_at + 1);
            v[write_at] = ack.clone();
            let mut f = fixture::setup(v).await;
            let e = f
                .client
                .native_reorder_playlist_tracks(&credential(), id, &request(None))
                .await
                .unwrap_err();
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert_eq!(e.details["write_requests_dispatched"], 1);
            assert!(!e.retryable);
            fixture::requests(&mut f, write_at + 1).await;
        }
        for after in [
            vec![11, 22, 22, 33],
            vec![22, 22, 11, 33],
            vec![22, 33, 11],
            vec![22, 33, 11, 11],
            vec![22, 33, 11, 22, 44],
        ] {
            let mut f =
                fixture::setup(custom_flow("42", favorite, &[11, 22, 22, 33], &after)).await;
            let e = f
                .client
                .native_reorder_playlist_tracks(&credential(), id, &request(None))
                .await
                .unwrap_err();
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            fixture::requests(&mut f, boundaries).await;
        }
    }
}

#[tokio::test]
async fn native_track_sort_preserves_all_known_song_metadata_including_duplicate_variants() {
    for favorite in [false, true] {
        for lose_variant in [false, true] {
            let (id, boundaries, write_at) = case(favorite);
            let mut v = flow("42", favorite);
            for (i, body) in v.iter_mut().enumerate() {
                edit(body, |d| {
                    if let Some(rows) = d["info"]["musiclist"].as_array_mut() {
                        if i < write_at {
                            rows[1]["name"] = json!("variant-A");
                            rows[2]["name"] = json!("variant-B");
                        } else {
                            rows[0]["name"] = json!(if lose_variant {
                                "variant-A"
                            } else {
                                "variant-B"
                            });
                            rows[3]["name"] = json!("variant-A");
                        }
                    }
                });
            }
            let mut f = fixture::setup(v).await;
            let result = f
                .client
                .native_reorder_playlist_tracks(&credential(), id, &request(None))
                .await;
            if lose_variant {
                let e = result.unwrap_err();
                assert_eq!(e.details["write_outcome"], "unconfirmed");
            } else {
                assert_eq!(result.unwrap().extensions["confirmed"], true);
            }
            fixture::requests(&mut f, boundaries).await;
        }
    }
}

#[tokio::test]
async fn native_track_sort_retains_playlist_metadata_and_supports_published_lists_without_withdrawal()
 {
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        for key in if favorite {
            vec!["info", "ispub", "turn"]
        } else {
            vec!["tag", "tagid", "igsl", "playlist_type"]
        } {
            let mut v = flow("42", favorite);
            for body in v.iter_mut().skip(write_at + 1) {
                edit(body, |d| {
                    if favorite {
                        if d.get("plist").is_some() {
                            d["plist"][0][key] = match key {
                                "ispub" => json!(true),
                                "turn" => json!(1),
                                _ => json!("changed"),
                            };
                        }
                    } else if d.get("sl_data").is_some() {
                        d["sl_data"][key] = match key {
                            "tag" => json!("摇滚,轻快"),
                            "tagid" => json!("999,998"),
                            "igsl" => json!(1),
                            _ => json!(3),
                        };
                    }
                });
            }
            let mut f = fixture::setup(v).await;
            let e = f
                .client
                .native_reorder_playlist_tracks(&credential(), id, &request(None))
                .await
                .unwrap_err();
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            fixture::requests(&mut f, boundaries).await;
        }
    }
    let mut v = flow("42", false);
    for body in &mut v {
        edit(body, |d| {
            if d.get("sl_data").is_some() {
                d["sl_data"]["igsl"] = json!(1);
            }
        });
    }
    let mut f = fixture::setup(v).await;
    assert!(
        f.client
            .native_reorder_playlist_tracks(&credential(), "101", &request(None))
            .await
            .is_ok()
    );
    assert!(
        fixture::requests(&mut f, 14)
            .await
            .iter()
            .all(|r| !r.contains("contribute") && !r.contains("pl3_editlist"))
    );
}

#[tokio::test]
async fn native_track_sort_large_full_permutation_and_regenerated_cover_are_confirmed() {
    for favorite in [false, true] {
        let mut before = vec![i64::MAX as u64; 9998];
        before.extend([22, 11]);
        let mut after = vec![11, 22];
        after.extend(vec![i64::MAX as u64; 9998]);
        let mut r = request(None);
        r.track_refs = refs(&after);
        let mut f = fixture::setup(custom_flow("42", favorite, &before, &after)).await;
        let result = f
            .client
            .native_reorder_playlist_tracks(&credential(), case(favorite).0, &r)
            .await
            .unwrap();
        assert_eq!(result.track_refs, r.track_refs);
        assert_eq!(result.extensions["cloud_track_count"], 10000);
        let count = case(favorite).1 + 36;
        let write_at = case(favorite).2 + 18;
        let seen = fixture::requests(&mut f, count).await;
        assert_eq!(payload(&seen[write_at])["data"], json!(after));
    }
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        let mut v = flow("42", favorite);
        for body in v.iter_mut().skip(write_at + 1) {
            edit(body, |d| {
                if d.get("plist").is_some() {
                    d["plist"][0]["pic"] = json!("https://img4.kuwo.cn/star/albumcover/new.jpg");
                }
                if d.get("sl_data").is_some() {
                    d["sl_data"]["pic"] = json!("https://img4.kuwo.cn/star/albumcover/new.jpg");
                    d["sl_data"]["big_pic"] = json!("https://img4.kuwo.cn/star/albumcover/new.jpg");
                }
            });
        }
        let mut f = fixture::setup(v).await;
        let result = f
            .client
            .native_reorder_playlist_tracks(&credential(), id, &request(None))
            .await
            .unwrap();
        assert_eq!(result.extensions["cover_changed"], true);
        fixture::requests(&mut f, boundaries).await;
    }
}
