use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        playlist::tests::{detail, directory, page},
        tests as fixture,
    },
};
use serde_json::Value;

pub(crate) fn request(
    account: Option<&str>,
    action: PlaylistItemMutationAction,
) -> PlaylistItemMutationRequest {
    PlaylistItemMutationRequest {
        item_refs: vec![
            ResourceRef::new(
                Platform::Kuwo,
                if action == PlaylistItemMutationAction::Add {
                    "44"
                } else {
                    "22"
                },
            )
            .unwrap(),
        ],
        kind: PlaylistItemKind::Track,
        account: account.map(str::to_owned),
    }
}
pub(crate) fn snapshot(uid: &str, ids: &[u64]) -> Vec<Vec<u8>> {
    let count = ids.len() as u64;
    let mut bodies = vec![
        json_response(&directory(Some(count))),
        json_response(&detail(uid, count)),
    ];
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
    bodies.extend([
        json_response(&detail(uid, count)),
        json_response(&directory(Some(count))),
    ]);
    bodies
}
pub(crate) fn flow(uid: &str, action: PlaylistItemMutationAction) -> Vec<Vec<u8>> {
    let after = if action == PlaylistItemMutationAction::Add {
        vec![44, 11, 22, 22, 33]
    } else {
        vec![11, 33]
    };
    custom_flow(uid, &[11, 22, 22, 33], &after)
}
fn custom_flow(uid: &str, before: &[u64], after: &[u64]) -> Vec<Vec<u8>> {
    let mut bodies = vec![json_response(&json!({"result":"ok"}))];
    bodies.extend(snapshot(uid, before));
    bodies.push(json_response(&json!({"errcode":0,"pid":101})));
    bodies.extend(snapshot(uid, after));
    bodies
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
fn refs(ids: &[u64]) -> Vec<ResourceRef> {
    ids.iter()
        .map(|id| ResourceRef::new(Platform::Kuwo, id.to_string()).unwrap())
        .collect()
}
fn payload(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn native_items_add_only_missing_tracks_and_preserve_existing_order_and_duplicates() {
    for after in [vec![44, 11, 55, 22, 22, 33], vec![11, 22, 22, 33, 55, 44]] {
        let mut r = request(None, PlaylistItemMutationAction::Add);
        r.item_refs = refs(&[22, 44, 55]);
        let mut f = fixture::setup(custom_flow("42", &[11, 22, 22, 33], &after)).await;
        let result = f
            .client
            .native_mutate_playlist_items(&credential(), "101", PlaylistItemMutationAction::Add, &r)
            .await
            .unwrap();
        assert_eq!(result.item_refs, r.item_refs);
        assert_eq!(result.cloud_track_count, Some(6));
        assert_eq!(result.action, PlaylistItemMutationAction::Add);
        assert!(
            result
                .snapshot_id
                .as_deref()
                .unwrap()
                .starts_with("kuwo-native_created_playlist-")
        );
        assert_eq!(result.extensions["sent_occurrences"], 2);
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(result.extensions["confirmed"], true);
        assert_eq!(result.extensions["atomic"], false);
        let seen = fixture::requests(&mut f, 14).await;
        assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 1);
        assert!(seen[7].contains("op=pl3_add&"));
        assert!(
            seen[7]
                .to_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        assert!(seen[7].contains("loginUid=42,loginSid=selected-session,"));
        assert!(!seen[7].to_lowercase().contains("\r\ncookie:"));
        assert_eq!(payload(&seen[7]), json!({"pid":101,"data":[44,55]}));
        assert!(
            seen.iter()
                .all(|r| !r.contains("pl3_sort") && !r.contains("ucheck"))
        );
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("selected-session")
        );
    }
}

#[tokio::test]
async fn native_items_remove_expands_each_target_occurrence_and_retains_other_duplicates() {
    let mut r = request(None, PlaylistItemMutationAction::Remove);
    r.item_refs = refs(&[22, 99]);
    let mut f = fixture::setup(custom_flow("42", &[11, 22, 22, 33, 33], &[11, 33, 33])).await;
    let result = f
        .client
        .native_mutate_playlist_items(&credential(), "101", PlaylistItemMutationAction::Remove, &r)
        .await
        .unwrap();
    assert_eq!(result.cloud_track_count, Some(3));
    assert_eq!(result.item_refs, r.item_refs);
    assert_eq!(result.extensions["sent_occurrences"], 2);
    let seen = fixture::requests(&mut f, 14).await;
    assert!(seen[7].contains("op=pl3_delete&"));
    assert_eq!(payload(&seen[7]), json!({"pid":101,"data":[22,22]}));
}

#[tokio::test]
async fn native_items_existing_additions_and_absent_removals_are_verified_noops() {
    for (action, ids) in [
        (PlaylistItemMutationAction::Add, vec![22]),
        (PlaylistItemMutationAction::Remove, vec![99]),
    ] {
        let mut bodies = vec![json_response(&json!({"result":"ok"}))];
        bodies.extend(snapshot("42", &[11, 22, 22, 33]));
        let mut r = request(None, action);
        r.item_refs = refs(&ids);
        let mut f = fixture::setup(bodies).await;
        let result = f
            .client
            .native_mutate_playlist_items(&credential(), "101", action, &r)
            .await
            .unwrap();
        assert_eq!(result.extensions["changed"], false);
        assert_eq!(result.extensions["write_requests_dispatched"], 0);
        assert_eq!(result.extensions["sent_occurrences"], 0);
        assert_eq!(result.cloud_track_count, Some(4));
        assert!(
            fixture::requests(&mut f, 7)
                .await
                .iter()
                .all(|r| r.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn native_items_validate_all_targets_kind_and_sdk_account_before_io() {
    let action = PlaylistItemMutationAction::Add;
    let mut f = fixture::setup(vec![]).await;
    for ids in [
        vec![],
        refs(&[11, 11]),
        refs(&(1..=101).collect::<Vec<_>>()),
        vec![ResourceRef::new(Platform::Soda, "11").unwrap()],
        vec![ResourceRef::new(Platform::Kuwo, "011").unwrap()],
        vec![ResourceRef::new(Platform::Kuwo, "9223372036854775808").unwrap()],
    ] {
        let mut r = request(None, action);
        r.item_refs = ids;
        assert_eq!(
            f.client
                .native_mutate_playlist_items(&credential(), "101", action, &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for id in ["0", "0101", "101&uid=7"] {
        assert_eq!(
            f.client
                .native_mutate_playlist_items(&credential(), id, action, &request(None, action))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.client
            .native_mutate_playlist_items(
                &credential(),
                "101",
                action,
                &request(Some("personal"), action)
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let mut r = request(None, action);
    r.kind = PlaylistItemKind::Video;
    assert_eq!(
        f.client
            .native_mutate_playlist_items(&credential(), "101", action, &r)
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    fixture::requests(&mut f, 0).await;
}

#[tokio::test]
async fn native_items_require_owned_ordinary_lists_and_do_not_withdraw_published_additions() {
    for action in [
        PlaylistItemMutationAction::Add,
        PlaylistItemMutationAction::Remove,
    ] {
        for id in ["901", "999"] {
            let mut f = fixture::setup(vec![
                json_response(&json!({"result":"ok"})),
                json_response(
                    &json!({"errcode":0,"plist":[{"id":901,"type":"MYFAVORITE","musicnum":0}]}),
                ),
            ])
            .await;
            assert_eq!(
                f.client
                    .native_mutate_playlist_items(&credential(), id, action, &request(None, action))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::PermissionDenied
            );
            fixture::requests(&mut f, 2).await;
        }
    }
    let action = PlaylistItemMutationAction::Add;
    let mut bodies = flow("42", action);
    let mut m = detail("42", 4);
    m["sl_data"]["igsl"] = json!("1");
    for at in [2, 5] {
        bodies[at] = json_response(&m);
    }
    bodies.truncate(7);
    let mut f = fixture::setup(bodies).await;
    let e = f
        .client
        .native_mutate_playlist_items(&credential(), "101", action, &request(None, action))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::CapabilityNotSupported);
    assert!(e.details.get("write_outcome").is_none());
    assert!(
        fixture::requests(&mut f, 7)
            .await
            .iter()
            .all(|r| r.starts_with("GET "))
    );
}

#[tokio::test]
async fn native_items_published_removal_keeps_ten_tracks_and_existing_addition_is_a_noop() {
    for action in [
        PlaylistItemMutationAction::Add,
        PlaylistItemMutationAction::Remove,
    ] {
        let before = [11, 22, 22, 33, 44, 55, 66, 77, 88, 99, 100, 101];
        let after = [11, 33, 44, 55, 66, 77, 88, 99, 100, 101];
        let mut bodies = custom_flow("42", &before, &after);
        for (at, count) in [(2, 12), (5, 12), (9, 10), (12, 10)] {
            let mut m = detail("42", count);
            m["sl_data"]["igsl"] = json!("1");
            bodies[at] = json_response(&m);
        }
        let mut r = request(None, action);
        let count = if action == PlaylistItemMutationAction::Add {
            r.item_refs = refs(&[22]);
            bodies.truncate(7);
            7
        } else {
            14
        };
        let mut f = fixture::setup(bodies).await;
        let result = f
            .client
            .native_mutate_playlist_items(&credential(), "101", action, &r)
            .await
            .unwrap();
        assert_eq!(
            result.extensions["changed"],
            action == PlaylistItemMutationAction::Remove
        );
        let seen = fixture::requests(&mut f, count).await;
        assert_eq!(
            seen.iter().filter(|r| r.starts_with("POST ")).count(),
            usize::from(action == PlaylistItemMutationAction::Remove)
        );
        assert!(
            seen.iter()
                .filter(|r| r.starts_with("POST "))
                .all(|r| r.contains("op=pl3_delete&"))
        );
    }
}

#[tokio::test]
async fn native_items_ack_and_complete_ordered_readback_must_match_without_retry_or_rollback() {
    for action in [
        PlaylistItemMutationAction::Add,
        PlaylistItemMutationAction::Remove,
    ] {
        for bad in [
            json_response(&json!({"errcode":603})),
            json_response(&json!({"errcode":0,"pid":102})),
            response(
                302,
                "application/json",
                "Location: https://evil.test/\r\n",
                b"{}",
            ),
            response(200, "text/html", "", b"{}"),
            response(200, "application/json", "", &vec![b' '; ACK_LIMIT + 1]),
        ] {
            let mut bodies = flow("42", action);
            bodies[7] = bad;
            bodies.truncate(8);
            let mut f = fixture::setup(bodies).await;
            let e = f
                .client
                .native_mutate_playlist_items(&credential(), "101", action, &request(None, action))
                .await
                .unwrap_err();
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            assert_eq!(
                fixture::requests(&mut f, 8)
                    .await
                    .iter()
                    .filter(|r| r.starts_with("POST "))
                    .count(),
                1
            );
        }
        let invalid_after = if action == PlaylistItemMutationAction::Add {
            vec![
                vec![11, 22, 22, 33],
                vec![44, 11, 22, 33],
                vec![44, 11, 22, 22, 33, 55],
                vec![44, 33, 22, 22, 11],
                vec![44, 44, 11, 22, 22, 33],
            ]
        } else {
            vec![vec![11, 22, 33], vec![33, 11], vec![11], vec![11, 33, 55]]
        };
        for after in invalid_after {
            let mut f = fixture::setup(custom_flow("42", &[11, 22, 22, 33], &after)).await;
            let e = f
                .client
                .native_mutate_playlist_items(&credential(), "101", action, &request(None, action))
                .await
                .unwrap_err();
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert_eq!(e.details["write_requests_dispatched"], 1);
            assert_eq!(e.details["automatic_retry"], false);
            fixture::requests(&mut f, 14).await;
        }
    }
}

#[tokio::test]
async fn native_items_verify_nontrack_metadata_and_report_platform_cover_changes() {
    for action in [
        PlaylistItemMutationAction::Add,
        PlaylistItemMutationAction::Remove,
    ] {
        let ids = if action == PlaylistItemMutationAction::Add {
            vec![44, 11, 22, 22, 33]
        } else {
            vec![11, 33]
        };
        for field in [
            "title",
            "desc",
            "tag",
            "tagid",
            "igsl",
            "playlist_type",
            "ispub",
            "cover",
        ] {
            let mut bodies = flow("42", action);
            let mut m = detail("42", ids.len() as u64);
            let mut d = directory(Some(ids.len() as u64));
            match field {
                "title" => {
                    m["sl_data"]["title"] = json!("changed");
                    d["plist"][0]["title"] = json!("changed");
                }
                "desc" => {
                    m["sl_data"]["desc"] = json!("changed");
                    d["plist"][0]["info"] = json!("changed");
                }
                "tag" => m["sl_data"]["tag"] = json!("其他,安静"),
                "tagid" => m["sl_data"]["tagid"] = json!("500,400"),
                "igsl" => m["sl_data"]["igsl"] = json!("1"),
                "playlist_type" => m["sl_data"]["playlist_type"] = json!(1),
                "ispub" => d["plist"][0]["ispub"] = json!(true),
                _ => {
                    let pic = "https://img4.kuwo.cn/star/albumcover/new.jpg";
                    d["plist"][0]["pic"] = json!(pic);
                    m["sl_data"]["pic"] = json!(pic);
                    m["sl_data"]["big_pic"] = json!(pic);
                }
            }
            for at in [8, 13] {
                bodies[at] = json_response(&d);
            }
            for at in [9, 12] {
                bodies[at] = json_response(&m);
            }
            let mut f = fixture::setup(bodies).await;
            let result = f
                .client
                .native_mutate_playlist_items(&credential(), "101", action, &request(None, action))
                .await;
            if field == "cover" {
                assert_eq!(result.unwrap().extensions["cover_changed"], true);
            } else {
                assert_eq!(result.unwrap_err().details["write_outcome"], "unconfirmed");
            }
            fixture::requests(&mut f, 14).await;
        }
    }
}

#[tokio::test]
async fn native_items_empty_transitions_and_large_duplicate_removal_remain_lossless() {
    for (action, before, after) in [
        (PlaylistItemMutationAction::Add, vec![], vec![44]),
        (PlaylistItemMutationAction::Remove, vec![22, 22], vec![]),
        (PlaylistItemMutationAction::Remove, vec![22; 1000], vec![]),
    ] {
        let mut f = fixture::setup(custom_flow("42", &before, &after)).await;
        let result = f
            .client
            .native_mutate_playlist_items(&credential(), "101", action, &request(None, action))
            .await
            .unwrap();
        assert_eq!(result.cloud_track_count, Some(after.len() as u64));
        let seen = fixture::requests(&mut f, 14).await;
        let data = payload(&seen[7]);
        assert_eq!(
            data["data"].as_array().unwrap().len(),
            if action == PlaylistItemMutationAction::Add {
                1
            } else {
                before.len()
            }
        );
    }
}

#[tokio::test]
async fn native_items_prevents_unverifiable_overflow_and_can_remove_all_ten_thousand_occurrences() {
    let rid = i64::MAX as u64;
    let before = vec![rid; playlist::MAX_TRACKS];
    let mut bodies = vec![json_response(&json!({"result":"ok"}))];
    bodies.extend(snapshot("42", &before));
    let count = bodies.len();
    let mut f = fixture::setup(bodies).await;
    let action = PlaylistItemMutationAction::Add;
    let e = f
        .client
        .native_mutate_playlist_items(&credential(), "101", action, &request(None, action))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    assert!(e.details.get("write_outcome").is_none());
    assert!(
        fixture::requests(&mut f, count)
            .await
            .iter()
            .all(|r| r.starts_with("GET "))
    );
    let mut f = fixture::setup(custom_flow("42", &before, &[])).await;
    let action = PlaylistItemMutationAction::Remove;
    let mut r = request(None, action);
    r.item_refs = refs(&[rid]);
    let result = f
        .client
        .native_mutate_playlist_items(&credential(), "101", action, &r)
        .await
        .unwrap();
    assert_eq!(result.cloud_track_count, Some(0));
    assert_eq!(result.extensions["sent_occurrences"], playlist::MAX_TRACKS);
    let seen = fixture::requests(&mut f, count + 7).await;
    let p = payload(&seen[count]);
    let data = p["data"].as_array().unwrap();
    assert_eq!(data.len(), playlist::MAX_TRACKS);
    assert!(data.iter().all(|id| id == &json!(rid)));
}

#[tokio::test]
async fn native_submission_repro_removal_cannot_implicitly_take_a_published_playlist_offline() {
    let before: Vec<_> = (1..=12).collect();
    let after: Vec<_> = (1..=9).collect();
    let mut bodies = custom_flow("42", &before, &after);
    for (at, count) in [(2, 12), (5, 12), (9, 9), (12, 9)] {
        let mut m = detail("42", count);
        m["sl_data"]["igsl"] = json!("1");
        bodies[at] = json_response(&m);
    }
    let mut r = request(None, PlaylistItemMutationAction::Remove);
    r.item_refs = refs(&[10, 11, 12]);
    bodies.truncate(7);
    let mut f = fixture::setup(bodies).await;
    let result = f
        .client
        .native_mutate_playlist_items(&credential(), "101", PlaylistItemMutationAction::Remove, &r)
        .await;
    assert_eq!(
        result.err().map(|e| e.code),
        Some(ErrorCode::CapabilityNotSupported)
    );
    let seen = fixture::requests(&mut f, 7).await;
    assert!(
        seen.iter()
            .all(|request| request.starts_with("GET ") && !request.contains("user_songlist_up"))
    );
}

pub(crate) fn published_flow(uid: &str, before: &[u64], after: &[u64]) -> Vec<Vec<u8>> {
    let mut bodies = custom_flow(uid, before, after);
    for (at, count) in [
        (2, before.len()),
        (5, before.len()),
        (9, after.len()),
        (12, after.len()),
    ] {
        let mut m = detail(uid, count as u64);
        m["sl_data"]["igsl"] = json!("1");
        bodies[at] = json_response(&m);
    }
    bodies
}

#[tokio::test]
async fn native_submission_removal_threshold_counts_each_duplicate_and_preserves_noops() {
    for (before, ids, allowed) in [
        ((1..=12).collect::<Vec<_>>(), vec![11, 12], true),
        ((1..=11).collect(), vec![10, 11], false),
        (vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 11], vec![11], true),
        (vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 10], vec![10], false),
        ((1..=9).collect(), vec![99], true),
        ((1..=9).collect(), vec![9], false),
    ] {
        let after: Vec<_> = before
            .iter()
            .copied()
            .filter(|id| !ids.contains(id))
            .collect();
        let changed = after != before;
        let expected_calls = if allowed && changed { 14 } else { 7 };
        let mut bodies = published_flow("42", &before, &after);
        bodies.truncate(expected_calls);
        let mut r = request(None, PlaylistItemMutationAction::Remove);
        r.item_refs = refs(&ids);
        let mut f = fixture::setup(bodies).await;
        let result = f
            .client
            .native_mutate_playlist_items(
                &credential(),
                "101",
                PlaylistItemMutationAction::Remove,
                &r,
            )
            .await;
        if allowed {
            let result = result.unwrap();
            assert_eq!(result.cloud_track_count, Some(after.len() as u64));
            assert_eq!(result.extensions["changed"], changed);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
            assert!(error.details.get("write_outcome").is_none());
        }
        let seen = fixture::requests(&mut f, expected_calls).await;
        assert_eq!(
            seen.iter().filter(|r| r.starts_with("POST ")).count(),
            usize::from(allowed && changed)
        );
        assert!(seen.iter().all(|r| !r.contains("user_songlist_up")));
    }
}

#[tokio::test]
async fn native_submission_removal_retained_count_does_not_replace_publication_readback() {
    let before: Vec<_> = (1..=12).collect();
    let after: Vec<_> = (1..=10).collect();
    let mut bodies = published_flow("42", &before, &after);
    // Even after retaining ten rows, unexpected upstream delisting is unconfirmed.
    bodies[9] = json_response(&detail("42", 10));
    bodies[12] = json_response(&detail("42", 10));
    let mut r = request(None, PlaylistItemMutationAction::Remove);
    r.item_refs = refs(&[11, 12]);
    let mut f = fixture::setup(bodies).await;
    let error = f
        .client
        .native_mutate_playlist_items(&credential(), "101", PlaylistItemMutationAction::Remove, &r)
        .await
        .unwrap_err();
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert!(!error.retryable);
    let seen = fixture::requests(&mut f, 14).await;
    assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 1);
}
