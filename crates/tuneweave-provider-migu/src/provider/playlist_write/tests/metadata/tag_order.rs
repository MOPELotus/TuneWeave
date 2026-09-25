use super::*;

const FIRST: (&str, &str) = ("1000001672", "流行");
const SECOND: (&str, &str) = ("1000001762", "国语");
const THIRD: (&str, &str) = ("1000001679", "摇滚");
const NEW: (&str, &str) = ("1000001680", "新标签 & + / %");

fn reorder_flow() -> Flow {
    with_tag_catalogue(
        tag_flow(&[FIRST, SECOND], &[&[SECOND], &[SECOND, FIRST]]),
        &[FIRST, SECOND],
    )
}

fn reorder_request(alias: &str) -> PlaylistUpdateRequest {
    PlaylistUpdateRequest {
        tags: Some(vec![SECOND.1.into(), FIRST.1.into()]),
        account: Some(alias.into()),
        ..Default::default()
    }
}

fn writes(f: &Flow) -> Vec<usize> {
    f.labels
        .iter()
        .enumerate()
        .filter_map(|(at, label)| (*label == "write").then_some(at))
        .collect()
}

#[tokio::test]
async fn native_playlist_tag_reorder_uses_verified_remove_append_with_selected_account() {
    for mode in ["default", "named", "caller"] {
        for batch in [false, true] {
            let mut f = reorder_flow();
            let final_write = writes(&f)[1];
            if batch {
                for (label, value) in f.labels.iter().zip(&mut f.values).skip(final_write + 1) {
                    if matches!(*label, "after_metadata" | "after_metadata_final") {
                        value["data"]["title"] = json!(TITLE);
                        value["data"]["summary"] = json!(DESCRIPTION);
                    }
                    if *label == "after_library" {
                        for item in value["list"].as_array_mut().unwrap() {
                            if item["musicListId"] == "77" {
                                item["title"] = json!(TITLE);
                            }
                        }
                    }
                }
            }
            let (mut p, seen) = server(wire(&f)).await;
            let (store, original, alias) = setup_mode(&mut p, mode);
            let mut request = reorder_request(alias);
            if batch {
                request.variant = PlaylistMetadataUpdateVariant::Batch;
                request.name = Some(TITLE.into());
                request.description = Some(DESCRIPTION.into());
            }
            let result = p.update_playlist("77", &request).await.unwrap();
            let playlist = result.playlist.as_ref().unwrap();
            assert_eq!(playlist.tags, [SECOND.1, FIRST.1]);
            assert_eq!(
                playlist.extensions["tag_items"],
                tag_value(&[SECOND, FIRST])
            );
            assert_eq!(result.extensions["atomic"], false);
            assert_eq!(
                result.extensions["confirmed_tag_removals"],
                json!([FIRST.1])
            );
            assert_eq!(
                result.extensions["confirmed_tag_additions"],
                json!([FIRST.1])
            );
            assert_eq!(result.extensions["existing_track_order_preserved"], true);
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                let update = p.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    format!("p{}", f.values.len() - 1)
                );
                assert!(!update.secret().contains("native-token-fixture"));
                assert!(p.take_response_credential().unwrap().is_none());
            } else {
                assert_eq!(
                    read(&store, alias).token(),
                    format!("p{}", f.values.len() - 1)
                );
            }
            let output = serde_json::to_string(&result).unwrap();
            assert!(!output.contains("native-token-fixture"));
            assert!(!output.contains("native-session-fixture"));
            let requests = seen.await.unwrap();
            assert_eq!(requests.len(), f.values.len());
            let posts = requests
                .iter()
                .filter(|r| r.starts_with("POST "))
                .collect::<Vec<_>>();
            assert_eq!(posts.len(), 2);
            for (i, post) in posts.iter().enumerate() {
                assert!(post.starts_with(&format!("POST {NATIVE_PATH} HTTP/1.1")));
                assert!(post.contains("token: native-token-fixture\r\n"));
                assert!(post.contains("signversion: V005\r\n"));
                assert!(!post.contains("pacmtoken:"));
                let body = post.split_once("\r\n\r\n").unwrap().1;
                let fields = url::form_urlencoded::parse(body.as_bytes())
                    .into_owned()
                    .collect::<BTreeMap<_, _>>();
                assert_eq!(fields["id"], "77");
                assert_eq!(fields["songflag"], "0");
                if i == 0 {
                    assert_eq!(fields["delTagIds"], FIRST.0);
                    assert_eq!(fields["delTagNames"], FIRST.1);
                    assert_eq!(fields.len(), 4);
                } else {
                    assert_eq!(fields["addTagIds"], format!("{}|", FIRST.0));
                    assert_eq!(fields["addTagNames"], format!("{}|", FIRST.1));
                    if batch {
                        assert_eq!(fields["title"], TITLE);
                        assert_eq!(fields["info"], DESCRIPTION);
                    }
                    assert_eq!(fields.len(), if batch { 6 } else { 4 });
                }
            }
        }
    }
}

#[tokio::test]
async fn native_playlist_tag_reorder_keeps_the_longest_valid_prefix_and_handles_new_labels() {
    for (desired, states, removed, added) in [
        (
            vec![SECOND, THIRD, FIRST],
            vec![vec![SECOND, THIRD], vec![SECOND, THIRD, FIRST]],
            vec![FIRST.1],
            vec![FIRST.1],
        ),
        (
            vec![SECOND, FIRST, THIRD],
            vec![
                vec![SECOND, THIRD],
                vec![SECOND],
                vec![SECOND, FIRST],
                vec![SECOND, FIRST, THIRD],
            ],
            vec![FIRST.1, THIRD.1],
            vec![FIRST.1, THIRD.1],
        ),
        (
            vec![NEW, SECOND],
            vec![
                vec![SECOND, THIRD],
                vec![THIRD],
                vec![],
                vec![NEW],
                vec![NEW, SECOND],
            ],
            vec![FIRST.1, SECOND.1, THIRD.1],
            vec![NEW.1, SECOND.1],
        ),
    ] {
        let slices = states.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let f = with_tag_catalogue(
            tag_flow(&[FIRST, SECOND, THIRD], &slices),
            &[FIRST, SECOND, THIRD, NEW],
        );
        let (mut p, seen) = server(wire(&f)).await;
        setup_mode(&mut p, "named");
        let result = p
            .update_playlist(
                "77",
                &PlaylistUpdateRequest {
                    tags: Some(desired.iter().map(|tag| tag.1.into()).collect()),
                    account: Some("personal".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            result.playlist.as_ref().unwrap().extensions["tag_items"],
            tag_value(&desired)
        );
        assert_eq!(result.extensions["confirmed_tag_removals"], json!(removed));
        assert_eq!(result.extensions["confirmed_tag_additions"], json!(added));
        assert_eq!(
            seen.await
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            states.len()
        );
    }
}

#[tokio::test]
async fn native_playlist_tag_reorder_requires_existing_identity_to_remain_in_official_catalogue() {
    for changed_id in [false, true] {
        let mut f = if changed_id {
            with_tag_catalogue(tag_flow(&[FIRST, SECOND], &[]), &[("99", FIRST.1), SECOND])
        } else {
            with_tag_catalogue(tag_flow(&[FIRST, SECOND], &[]), &[SECOND])
        };
        f.truncate(f.at("tag_catalogue") + 1);
        let (mut p, seen) = server(wire(&f)).await;
        setup_mode(&mut p, "named");
        let failure = p
            .update_playlist("77", &reorder_request("personal"))
            .await
            .unwrap_err();
        assert_eq!(
            failure.code,
            if changed_id {
                ErrorCode::UpstreamError
            } else {
                ErrorCode::InvalidRequest
            }
        );
        assert!(failure.details.get("write_outcome").is_none());
        assert!(seen.await.unwrap().iter().all(|r| !r.starts_with("POST ")));
    }
}

#[tokio::test]
async fn native_playlist_tag_reorder_reports_partial_removal_without_retry_or_rollback() {
    for stale_readback in [false, true] {
        let mut f = reorder_flow();
        let final_write = writes(&f)[1];
        if stale_readback {
            for (label, value) in f.labels.iter().zip(&mut f.values).skip(final_write + 1) {
                if matches!(*label, "after_metadata" | "after_metadata_final") {
                    value["data"]["tags"] = tag_value(&[SECOND]);
                }
            }
            f.truncate(f.at("after_library"));
        } else {
            f.values[final_write] = json!({"code":"200013","info":"native-token-fixture"});
            f.truncate(final_write + 1);
        }
        let (mut p, seen) = server(wire(&f)).await;
        setup_mode(&mut p, "caller");
        let failure = p
            .update_playlist("77", &reorder_request("default"))
            .await
            .unwrap_err();
        assert!(!failure.retryable);
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert_eq!(failure.details["confirmed_tag_removals"], json!([FIRST.1]));
        assert_eq!(failure.details["confirmed_tag_additions"], json!([]));
        assert!(!failure.details.to_string().contains("native-token-fixture"));
        assert_eq!(
            seen.await
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            2
        );
    }
}

#[tokio::test]
async fn native_playlist_tag_reorder_does_not_continue_after_replacement_or_cancel() {
    for cancel in [false, true] {
        let mut f = reorder_flow();
        let final_write = writes(&f)[1];
        f.truncate(final_write + 1);
        let (mut p, seen, release, server_task) = gated(wire(&f)).await;
        let (store, original, alias) = setup_mode(&mut p, if cancel { "caller" } else { "named" });
        let provider = p.clone();
        let task = tokio::spawn(async move {
            provider
                .update_playlist("77", &reorder_request(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .unwrap()
            .unwrap();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(p.take_response_credential().unwrap().is_none());
            assert_eq!(read(&store, alias), original);
            server_task.abort();
        } else {
            let replacement =
                MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
            store.put(&stored(alias, &replacement)).unwrap();
            release.send(()).unwrap();
            let failure = task.await.unwrap().unwrap_err();
            assert_eq!(failure.code, ErrorCode::Conflict);
            assert_eq!(failure.details["confirmed_tag_removals"], json!([FIRST.1]));
            assert!(!failure.retryable);
            assert_eq!(read(&store, alias), replacement);
            tokio::time::timeout(Duration::from_secs(5), server_task)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
