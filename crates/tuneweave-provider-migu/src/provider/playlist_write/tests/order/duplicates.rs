use super::*;

#[tokio::test]
async fn native_playlist_order_duplicates_confirm_each_move_and_report_partial_failure() {
    let original = vec![1, 2, 1, 2, 3];
    let stages = vec![vec![3, 1, 2, 1, 2], vec![3, 2, 1, 2, 1]];
    for fail_second in [false, true] {
        let mut f = flow(&original, &stages);
        if fail_second {
            let at = f
                .labels
                .iter()
                .enumerate()
                .filter(|(_, label)| **label == "move")
                .nth(1)
                .unwrap()
                .0;
            f.values[at] = json!({"code":"200013"});
            f.truncate(at + 1);
        }
        let (mut provider, seen) = server(wire(&f)).await;
        setup_mode(&mut provider, "named");
        let result = provider
            .reorder_playlist_tracks("77", &request(&stages[1], "personal"))
            .await;
        let details = if fail_second {
            let failure = result.unwrap_err();
            assert_eq!(failure.code, ErrorCode::PermissionDenied);
            assert_eq!(failure.details["write_outcome"], "unconfirmed");
            assert!(!failure.retryable);
            failure.details
        } else {
            let result = result.unwrap();
            assert_eq!(
                result.track_refs,
                request(&stages[1], "personal").track_refs
            );
            assert_eq!(result.extensions["atomic"], false);
            json!(result.extensions)
        };
        assert_eq!(details["moves_dispatched"], 2);
        assert_eq!(
            details["confirmed_moves"].as_array().unwrap().len(),
            if fail_second { 1 } else { 2 }
        );
        assert_eq!(details["confirmed_moves"][0]["old_position"], 5);
        assert_eq!(details["confirmed_moves"][0]["new_position"], 1);
        if !fail_second {
            assert_eq!(details["confirmed_moves"][1]["old_position"], 5);
            assert_eq!(details["confirmed_moves"][1]["new_position"], 2);
        }
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), f.values.len());
        assert_eq!(
            requests.iter().filter(|r| r.contains(ORDER_PATH)).count(),
            2
        );
    }
}

#[tokio::test]
async fn native_playlist_order_duplicates_use_minimum_positions_and_preserve_all_occurrences() {
    let mut across = (1..=51).collect::<Vec<_>>();
    across.push(1);
    let mut front = across.clone();
    front.rotate_right(1);
    for (original, desired) in [
        (vec![1, 2, 1, 2], vec![2, 1, 2, 1]),
        (across.clone(), front.clone()),
        (front, across),
    ] {
        for mode in ["default", "named", "caller"] {
            let f = flow(&original, std::slice::from_ref(&desired));
            let (mut provider, seen) = server(wire(&f)).await;
            let (store, initial, alias) = setup_mode(&mut provider, mode);
            let requested = request(&desired, alias);
            let result = provider
                .reorder_playlist_tracks("77", &requested)
                .await
                .unwrap();
            assert_eq!(result.track_refs, requested.track_refs);
            assert_eq!(result.extensions["moves_dispatched"], 1);
            assert_eq!(result.extensions["atomic"], true);
            let confirmed = &result.extensions["confirmed_moves"][0];
            let from = confirmed["old_position"].as_u64().unwrap() as usize - 1;
            let to = confirmed["new_position"].as_u64().unwrap() as usize - 1;
            assert_eq!(confirmed["content_id"], original[from].to_string());
            assert_eq!(confirmed["song_id"], format!("s{}", original[from]));
            let mut actual = original.clone();
            let moved = actual.remove(from);
            actual.insert(to, moved);
            assert_eq!(actual, desired);
            let requests = seen.await.unwrap();
            assert_eq!(requests.len(), f.values.len());
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
            assert_eq!(fields["oldPostion"], (from + 1).to_string());
            assert_eq!(fields["newPosition"], (to + 1).to_string());
            assert_eq!(fields["contentId"], original[from].to_string());
            assert_eq!(fields["songId"], format!("s{}", original[from]));
            assert!(moves[0].contains("token: native-token-fixture\r\n"));
            assert!(moves[0].contains("signversion: V005\r\n"));
            assert!(!moves[0].contains("pacmtoken:"));
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if mode == "caller" {
                assert_eq!(read(&store, alias), initial);
                let credential = provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&credential).unwrap().token(),
                    format!("p{}", f.values.len() - 1)
                );
                assert!(!credential.secret().contains("native-token-fixture"));
                assert!(provider.take_response_credential().unwrap().is_none());
            } else {
                assert_eq!(
                    read(&store, alias).token(),
                    format!("p{}", f.values.len() - 1)
                );
            }
            assert!(
                !serde_json::to_string(&result)
                    .unwrap()
                    .contains("native-token-fixture")
            );
        }
    }
}

#[tokio::test]
async fn native_playlist_order_duplicates_noop_still_reads_complete_owned_state() {
    let original = vec![1, 2, 1, 2];
    let f = flow(&original, &[]);
    let (mut provider, seen) = server(wire(&f)).await;
    setup_mode(&mut provider, "named");
    let requested = request(&original, "personal");
    let result = provider
        .reorder_playlist_tracks("77", &requested)
        .await
        .unwrap();
    assert_eq!(result.track_refs, requested.track_refs);
    assert_eq!(result.extensions["moves_dispatched"], 0);
    let requests = seen.await.unwrap();
    assert_eq!(requests.len(), f.values.len());
    assert!(
        !requests
            .iter()
            .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
    );
}

#[tokio::test]
async fn native_playlist_order_duplicates_reject_ambiguous_dtos_or_changed_counts_before_write() {
    for variant in 0..5 {
        let original = vec![1, 2, 1];
        let mut desired = vec![1, 1, 2];
        let mut f = flow(&original, std::slice::from_ref(&desired));
        if variant == 0 {
            desired = vec![1, 2, 2];
        } else {
            let at = f.at("before_tracks");
            match variant {
                1 => f.values[at]["data"]["songList"][2]["songId"] = json!("different"),
                2 => f.values[at]["data"]["songList"][2]["songName"] = json!("Different name"),
                3 => f.values[at]["data"]["songList"][2]["singerName"] = json!("Different singer"),
                _ => f.values[at]["data"]["songList"][1]["songId"] = json!("s1"),
            }
        }
        f.truncate(f.at("native_profile"));
        let (mut provider, seen) = server(wire(&f)).await;
        setup_mode(&mut provider, "named");
        let failure = provider
            .reorder_playlist_tracks("77", &request(&desired, "personal"))
            .await
            .unwrap_err();
        assert_eq!(
            failure.code,
            if variant == 0 {
                ErrorCode::InvalidRequest
            } else {
                ErrorCode::CapabilityNotSupported
            }
        );
        assert!(failure.details.get("write_outcome").is_none());
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), f.values.len());
        assert!(
            !requests
                .iter()
                .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
        );
    }
}

#[tokio::test]
async fn native_playlist_order_duplicates_unconfirmed_ack_or_readback_never_retries() {
    for variant in 0..4 {
        let original = vec![1, 2, 1, 2];
        let desired = vec![2, 1, 2, 1];
        let mut f = flow(&original, std::slice::from_ref(&desired));
        if variant == 0 {
            let at = f.at("move");
            f.values[at] = json!({"code":"200013","info":"native-token-fixture"});
            f.truncate(at + 1);
        } else {
            let at = f.at("after_tracks");
            match variant {
                1 => f.values[at]["data"]["songList"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 1),
                2 => {
                    f.values[at]["data"]["songList"][3]["songName"] =
                        json!("Changed duplicate metadata")
                }
                _ => {
                    f.values[at]["data"]["songList"][3] =
                        f.values[at]["data"]["songList"][0].clone()
                }
            }
            f.truncate(f.at("after_library"));
        }
        let (mut provider, seen) = server(wire(&f)).await;
        setup_mode(&mut provider, "named");
        let failure = provider
            .reorder_playlist_tracks("77", &request(&desired, "personal"))
            .await
            .unwrap_err();
        // Changed native fields may themselves fail the conservative DTO
        // eligibility check, but every post-dispatch error stays unconfirmed.
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert_eq!(failure.details["moves_dispatched"], 1);
        assert_eq!(failure.details["confirmed_moves"], json!([]));
        assert!(!failure.retryable);
        assert!(!format!("{failure:?}").contains("native-token-fixture"));
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), f.values.len());
        assert_eq!(
            requests.iter().filter(|r| r.contains(ORDER_PATH)).count(),
            1
        );
    }
}

#[tokio::test]
async fn native_playlist_order_duplicates_preserve_generation_and_cancelled_caller_boundary() {
    let original = vec![1, 2, 1, 2];
    let desired = vec![2, 1, 2, 1];
    for cancel in [false, true] {
        for boundary in ["move", "after_metadata"] {
            let mut f = flow(&original, std::slice::from_ref(&desired));
            f.truncate(f.at(boundary) + 1);
            let (mut provider, seen, release, server_task) = gated(wire(&f)).await;
            let (store, initial, alias) =
                setup_mode(&mut provider, if cancel { "caller" } else { "named" });
            let provider = Arc::new(provider);
            let running = provider.clone();
            let requested = request(&desired, alias);
            let task =
                tokio::spawn(
                    async move { running.reorder_playlist_tracks("77", &requested).await },
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
                assert_eq!(failure.details["confirmed_moves"], json!([]));
                assert!(!failure.retryable);
                assert_eq!(read(&store, alias), replacement);
                server_task.await.unwrap();
            }
        }
    }
}
