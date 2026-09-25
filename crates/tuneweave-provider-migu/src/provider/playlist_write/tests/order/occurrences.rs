use super::*;
use tuneweave_core::{PlaylistOccurrenceOrderRequest, PlaylistTrackOccurrence};

#[tokio::test]
async fn native_playlist_occurrences_validate_requests_before_account_or_network() {
    let (provider, seen) = server(vec![]).await;
    for page in [
        PageRequest::new(0, 0),
        PageRequest::new(101, 0),
        PageRequest::new(100, u32::MAX),
    ] {
        assert_eq!(
            provider
                .playlist_track_occurrences("77", &page)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for id in ["", "77?other=1"] {
        assert_eq!(
            provider
                .playlist_track_occurrences(id, &PageRequest::new(10, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (snapshot_id, occurrence_ids) in [
        (String::new(), vec![]),
        (
            format!("migu_occurrence_snapshot_v1_{}", "a".repeat(40)),
            vec!["bad-id".into()],
        ),
        (
            format!("migu_occurrence_snapshot_v1_{}", "a".repeat(40)),
            vec![format!("migu_occurrence_v1_{}", "b".repeat(40)); 2],
        ),
        (
            format!("migu_occurrence_snapshot_v1_{}", "a".repeat(40)),
            (0..10001)
                .map(|n| format!("migu_occurrence_v1_{n:040x}"))
                .collect(),
        ),
    ] {
        let request = PlaylistOccurrenceOrderRequest {
            snapshot_id,
            occurrence_ids,
            account: None,
        };
        assert_eq!(
            provider
                .reorder_playlist_occurrences("77", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(seen.await.unwrap().is_empty());
}

#[tokio::test]
async fn native_playlist_occurrences_reject_foreign_owner_before_publishing_ids_or_writing() {
    let original = vec![1, 2, 3];
    for write in [false, true] {
        let read_flow = occurrence_flow(&original, &[]);
        let mut foreign = occurrence_flow(&original, &[vec![3, 1, 2]]);
        for (label, value) in foreign.labels.iter().zip(&mut foreign.values) {
            if matches!(
                *label,
                "before_metadata" | "before_metadata_final" | "before_tracks"
            ) {
                value["data"]["ownerId"] = json!("222");
            }
        }
        foreign.truncate(foreign.at("native_profile"));
        let frames = if write {
            [wire(&read_flow), wire(&foreign)].concat()
        } else {
            wire(&foreign)
        };
        let expected_requests = frames.len();
        let (mut provider, seen) = server(frames).await;
        setup_mode(&mut provider, "named");
        let failure = if write {
            let page = read_occurrences(&provider, "personal").await;
            provider
                .reorder_playlist_occurrences("77", &order_request(&page, &[2, 0, 1], "personal"))
                .await
                .unwrap_err()
        } else {
            provider
                .playlist_track_occurrences(
                    "77",
                    &PageRequest {
                        limit: 100,
                        offset: 0,
                        account: Some("personal".into()),
                    },
                )
                .await
                .unwrap_err()
        };
        assert_eq!(failure.code, ErrorCode::PermissionDenied);
        assert!(failure.details.get("write_outcome").is_none());
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), expected_requests);
        assert!(
            !requests
                .iter()
                .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
        );
    }
}

#[tokio::test]
async fn native_playlist_occurrences_reject_cross_alias_replay_and_excessive_moves() {
    for cross_alias in [true, false] {
        let original = if cross_alias {
            vec![1, 2, 3]
        } else {
            (1..=18).collect::<Vec<_>>()
        };
        let reversed = original.iter().rev().copied().collect::<Vec<_>>();
        let read_flow = occurrence_flow(&original, &[]);
        let mut write_flow = occurrence_flow(&original, std::slice::from_ref(&reversed));
        write_flow.truncate(write_flow.at("native_profile"));
        let (mut provider, seen) = server([wire(&read_flow), wire(&write_flow)].concat()).await;
        setup_mode(&mut provider, "named");
        let page =
            read_occurrences(&provider, if cross_alias { "default" } else { "personal" }).await;
        let positions = (0..original.len()).rev().collect::<Vec<_>>();
        let failure = provider
            .reorder_playlist_occurrences("77", &order_request(&page, &positions, "personal"))
            .await
            .unwrap_err();
        assert_eq!(
            failure.code,
            if cross_alias {
                ErrorCode::Conflict
            } else {
                ErrorCode::CapabilityNotSupported
            }
        );
        assert!(failure.details.get("write_outcome").is_none());
        let requests = seen.await.unwrap();
        assert_eq!(
            requests.len(),
            read_flow.values.len() + write_flow.values.len()
        );
        assert!(
            !requests
                .iter()
                .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
        );
    }
}

fn occurrence_flow(before: &[u32], after: &[Vec<u32>]) -> Flow {
    let mut f = flow(before, after);
    // Labels 1 and 3 are different original Song DTOs for the same content ID.
    for (label, value) in f.labels.iter().zip(&mut f.values) {
        if matches!(*label, "before_tracks" | "after_tracks") {
            for song in value["data"]["songList"].as_array_mut().unwrap() {
                if song["contentId"] == "3" {
                    song["contentId"] = json!("1");
                }
            }
        }
    }
    f
}

async fn read_occurrences(provider: &MiguProvider, alias: &str) -> Page<PlaylistTrackOccurrence> {
    provider
        .playlist_track_occurrences(
            "77",
            &PageRequest {
                limit: 100,
                offset: 0,
                account: Some(alias.into()),
            },
        )
        .await
        .unwrap()
}

fn order_request(
    page: &Page<PlaylistTrackOccurrence>,
    positions: &[usize],
    alias: &str,
) -> PlaylistOccurrenceOrderRequest {
    PlaylistOccurrenceOrderRequest {
        occurrence_ids: positions
            .iter()
            .map(|position| page.items[*position].id.clone())
            .collect(),
        snapshot_id: page.pagination.extensions["source_snapshot_id"]
            .as_str()
            .unwrap()
            .into(),
        account: Some(alias.into()),
    }
}

#[tokio::test]
async fn native_playlist_occurrences_complete_snapshot_pages_are_stable_across_rotation() {
    let original = (1..=51).collect::<Vec<_>>();
    let f = occurrence_flow(&original, &[]);
    for mode in ["default", "named", "caller"] {
        let (mut provider, seen) = server([wire(&f), wire(&f)].concat()).await;
        let (store, initial, alias) = setup_mode(&mut provider, mode);
        assert!(
            provider
                .capabilities()
                .contains(&Capability::PlaylistOccurrenceRead)
        );
        assert!(
            provider
                .capabilities()
                .contains(&Capability::PlaylistOccurrenceWrite)
        );
        let first = provider
            .playlist_track_occurrences(
                "77",
                &PageRequest {
                    limit: 50,
                    offset: 0,
                    account: Some(alias.into()),
                },
            )
            .await
            .unwrap();
        if mode == "caller" {
            let credential = provider.take_response_credential().unwrap().unwrap();
            provider = provider.caller_scope(&credential).unwrap();
        }
        let last = provider
            .playlist_track_occurrences(
                "77",
                &PageRequest {
                    limit: 50,
                    offset: 50,
                    account: Some(alias.into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(first.pagination.total, Some(51));
        assert_eq!(first.pagination.next_offset, Some(50));
        assert_eq!(last.items.len(), 1);
        assert_eq!(last.items[0].position, 50);
        assert!(!last.pagination.has_more);
        assert_eq!(
            first.pagination.extensions["source_snapshot_id"],
            last.pagination.extensions["source_snapshot_id"]
        );
        assert_eq!(first.items[0].track.as_ref().unwrap().id, "1");
        assert_eq!(first.items[2].track.as_ref().unwrap().id, "1");
        assert_ne!(first.items[0].id, first.items[2].id);
        assert_eq!(
            first
                .items
                .iter()
                .map(|item| &item.id)
                .collect::<BTreeSet<_>>()
                .len(),
            50
        );
        assert!(
            !serde_json::to_string(&first)
                .unwrap()
                .contains("initial-pacm")
        );
        assert!(
            !serde_json::to_string(&first)
                .unwrap()
                .contains("native-token-fixture")
        );
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), initial);
        }
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), 2 * f.values.len());
        assert!(
            !requests
                .iter()
                .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
        );
    }
}

#[tokio::test]
async fn native_playlist_occurrences_roundtrip_moves_heterogeneous_duplicate_dto() {
    let original = vec![1, 2, 3];
    let read_flow = occurrence_flow(&original, &[]);
    let write_flow = occurrence_flow(&original, &[vec![3, 1, 2]]);
    for mode in ["default", "named", "caller"] {
        let (mut provider, seen) = server([wire(&read_flow), wire(&write_flow)].concat()).await;
        let (store, initial, alias) = setup_mode(&mut provider, mode);
        let page = read_occurrences(&provider, alias).await;
        let request = order_request(&page, &[2, 0, 1], alias);
        if mode == "caller" {
            let credential = provider.take_response_credential().unwrap().unwrap();
            provider = provider.caller_scope(&credential).unwrap();
        }
        let result = provider
            .reorder_playlist_occurrences("77", &request)
            .await
            .unwrap();
        assert_eq!(result.occurrence_ids.len(), 3);
        assert_ne!(result.snapshot_id, request.snapshot_id);
        assert_ne!(result.occurrence_ids, request.occurrence_ids);
        assert_eq!(result.extensions["moves_dispatched"], 1);
        assert_eq!(
            result.extensions["confirmed_moves"][0]["occurrence_id"],
            page.items[2].id
        );
        assert_eq!(result.extensions["confirmed_moves"][0]["old_position"], 3);
        assert_eq!(result.extensions["confirmed_moves"][0]["new_position"], 1);
        let requests = seen.await.unwrap();
        assert_eq!(
            requests.len(),
            read_flow.values.len() + write_flow.values.len()
        );
        let moves = requests
            .iter()
            .filter(|r| r.contains(ORDER_PATH))
            .collect::<Vec<_>>();
        assert_eq!(moves.len(), 1);
        let query = moves[0]
            .lines()
            .next()
            .unwrap()
            .split_once('?')
            .unwrap()
            .1
            .strip_suffix(" HTTP/1.1")
            .unwrap();
        let fields = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(fields["contentId"], "1");
        assert_eq!(fields["songId"], "s3");
        assert_eq!(fields["songName"], "Song 3");
        assert_eq!(fields["oldPostion"], "3");
        assert_eq!(fields["newPosition"], "1");
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), initial);
            let credential = provider.take_response_credential().unwrap().unwrap();
            assert!(!credential.secret().contains("native-token-fixture"));
            assert!(provider.take_response_credential().unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn native_playlist_occurrences_reject_stale_raw_fields_forgery_and_omissions_before_native() {
    for variant in 0..5 {
        let original = vec![1, 2, 3];
        let read_flow = occurrence_flow(&original, &[]);
        let mut write_flow = occurrence_flow(&original, &[vec![3, 1, 2]]);
        if variant < 2 {
            let at = write_flow.at("before_tracks");
            write_flow.values[at]["data"]["songList"][2][if variant == 0 {
                "singerName"
            } else {
                "songName"
            }] = json!("Changed original write DTO");
        }
        write_flow.truncate(write_flow.at("native_profile"));
        let (mut provider, seen) = server([wire(&read_flow), wire(&write_flow)].concat()).await;
        setup_mode(&mut provider, "named");
        let page = read_occurrences(&provider, "personal").await;
        let mut request = order_request(&page, &[2, 0, 1], "personal");
        match variant {
            2 => request.occurrence_ids[0] = format!("migu_occurrence_v1_{}", "0".repeat(40)),
            3 => {
                request.occurrence_ids.pop();
            }
            4 => request.snapshot_id = format!("migu_occurrence_snapshot_v1_{}", "0".repeat(40)),
            _ => {}
        }
        let failure = provider
            .reorder_playlist_occurrences("77", &request)
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::Conflict);
        assert!(failure.details.get("write_outcome").is_none());
        let requests = seen.await.unwrap();
        assert_eq!(
            requests.len(),
            read_flow.values.len() + write_flow.values.len()
        );
        assert!(
            !requests
                .iter()
                .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
        );
    }
}

#[tokio::test]
async fn native_playlist_occurrences_reject_duplicate_ids_before_io_and_identical_dto_swaps() {
    let original = vec![1, 2, 1];
    let f = flow(&original, &[]);
    let mut preflight = flow(&original, std::slice::from_ref(&original));
    preflight.truncate(preflight.at("native_profile"));
    let (mut provider, seen) = server([wire(&f), wire(&preflight)].concat()).await;
    setup_mode(&mut provider, "named");
    let page = read_occurrences(&provider, "personal").await;
    let mut request = order_request(&page, &[0, 0, 2], "personal");
    assert_eq!(
        provider
            .reorder_playlist_occurrences("77", &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    request = order_request(&page, &[2, 1, 0], "personal");
    let failure = provider
        .reorder_playlist_occurrences("77", &request)
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::CapabilityNotSupported);
    assert!(failure.details.get("write_outcome").is_none());
    assert_eq!(
        seen.await.unwrap().len(),
        f.values.len() + preflight.values.len()
    );
}

#[tokio::test]
async fn native_playlist_occurrences_noop_and_empty_playlist_require_complete_confirmation() {
    for original in [vec![], vec![1, 2, 3]] {
        let f = occurrence_flow(&original, &[]);
        let (mut provider, seen) = server([wire(&f), wire(&f)].concat()).await;
        setup_mode(&mut provider, "named");
        let page = read_occurrences(&provider, "personal").await;
        let positions = (0..original.len()).collect::<Vec<_>>();
        let request = order_request(&page, &positions, "personal");
        let result = provider
            .reorder_playlist_occurrences("77", &request)
            .await
            .unwrap();
        assert_eq!(result.snapshot_id, request.snapshot_id);
        assert_eq!(result.occurrence_ids, request.occurrence_ids);
        assert_eq!(result.extensions["moves_dispatched"], 0);
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), 2 * f.values.len());
        assert!(
            !requests
                .iter()
                .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
        );
    }
}

#[tokio::test]
async fn native_playlist_occurrences_relogin_invalidates_snapshot_before_dispatch() {
    let original = vec![1, 2, 3];
    let read_flow = occurrence_flow(&original, &[]);
    let mut write_flow = occurrence_flow(&original, &[vec![3, 1, 2]]);
    write_flow.truncate(write_flow.at("native_profile"));
    let (mut provider, seen) = server([wire(&read_flow), wire(&write_flow)].concat()).await;
    let (store, _, alias) = setup_mode(&mut provider, "named");
    let page = read_occurrences(&provider, alias).await;
    let request = order_request(&page, &[2, 0, 1], alias);
    let replacement = MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
    store.put(&stored(alias, &replacement)).unwrap();
    let failure = provider
        .reorder_playlist_occurrences("77", &request)
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::Conflict);
    assert!(failure.details.get("write_outcome").is_none());
    assert!(read(&store, alias).same_login(&replacement));
    let requests = seen.await.unwrap();
    assert_eq!(
        requests.len(),
        read_flow.values.len() + write_flow.values.len()
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
    );
}

#[tokio::test]
async fn native_playlist_occurrences_verify_every_dto_and_report_partial_ack_failure() {
    let original = vec![1, 2, 3, 4];
    let stages = vec![vec![4, 1, 2, 3], vec![4, 3, 1, 2]];
    for variant in 0..3 {
        let read_flow = occurrence_flow(&original, &[]);
        let mut write_flow = occurrence_flow(&original, &stages);
        let second = write_flow
            .labels
            .iter()
            .enumerate()
            .filter(|(_, label)| **label == "move")
            .nth(1)
            .unwrap()
            .0;
        if variant == 1 {
            write_flow.values[second] = json!({"code":"200013"});
            write_flow.truncate(second + 1);
        } else if variant == 2 {
            let at = write_flow.at("after_tracks");
            // Same normalized content ID at both positions, wrong native DTO order.
            write_flow.values[at]["data"]["songList"]
                .as_array_mut()
                .unwrap()
                .swap(1, 3);
            write_flow.truncate(second);
        }
        let (mut provider, seen) = server([wire(&read_flow), wire(&write_flow)].concat()).await;
        setup_mode(&mut provider, "named");
        let page = read_occurrences(&provider, "personal").await;
        let request = order_request(&page, &[3, 2, 0, 1], "personal");
        let result = provider.reorder_playlist_occurrences("77", &request).await;
        if variant == 0 {
            let result = result.unwrap();
            assert_eq!(
                result.extensions["confirmed_moves"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(result.extensions["moves_dispatched"], 2);
        } else {
            let failure = result.unwrap_err();
            assert_eq!(failure.details["write_outcome"], "unconfirmed");
            assert_eq!(
                failure.details["moves_dispatched"],
                if variant == 1 { 2 } else { 1 }
            );
            assert_eq!(
                failure.details["confirmed_moves"].as_array().unwrap().len(),
                usize::from(variant == 1)
            );
            assert!(!failure.retryable);
        }
        let requests = seen.await.unwrap();
        assert_eq!(
            requests.len(),
            read_flow.values.len() + write_flow.values.len()
        );
        assert_eq!(
            requests.iter().filter(|r| r.contains(ORDER_PATH)).count(),
            if variant == 2 { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn native_playlist_occurrences_preserve_original_login_and_cancelled_caller_state() {
    let original = vec![1, 2, 3];
    for cancel in [false, true] {
        for boundary in ["move", "after_metadata"] {
            let read_flow = occurrence_flow(&original, &[]);
            let mut write_flow = occurrence_flow(&original, &[vec![3, 1, 2]]);
            write_flow.truncate(write_flow.at(boundary) + 1);
            let (mut provider, seen, release, server_task) =
                gated([wire(&read_flow), wire(&write_flow)].concat()).await;
            let (store, initial, alias) =
                setup_mode(&mut provider, if cancel { "caller" } else { "named" });
            let page = read_occurrences(&provider, alias).await;
            let request = order_request(&page, &[2, 0, 1], alias);
            if cancel {
                let credential = provider.take_response_credential().unwrap().unwrap();
                provider = provider.caller_scope(&credential).unwrap();
            }
            let provider = Arc::new(provider);
            let running = provider.clone();
            let task =
                tokio::spawn(
                    async move { running.reorder_playlist_occurrences("77", &request).await },
                );
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert!(provider.take_response_credential().unwrap().is_none());
                assert_eq!(read(&store, alias), initial);
                drop(release);
                server_task.abort();
            } else {
                let replacement =
                    MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
                store.put(&stored(alias, &replacement)).unwrap();
                release.send(()).unwrap();
                let failure = task.await.unwrap().unwrap_err();
                assert_eq!(failure.code, ErrorCode::Conflict);
                assert_eq!(failure.details["write_outcome"], "unconfirmed");
                assert_eq!(failure.details["moves_dispatched"], 1);
                assert!(!failure.retryable);
                assert_eq!(read(&store, alias), replacement);
                server_task.await.unwrap();
            }
        }
    }
}
