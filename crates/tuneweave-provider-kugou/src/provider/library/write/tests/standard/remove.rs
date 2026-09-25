use super::*;

const PATH: &str = "POST /cloudlist.service/v4/delete_songs?";

fn full_snapshot(rows: Vec<Value>, version: u64) -> Vec<Frame> {
    let mut list = list_row(37, 0);
    list["is_def"] = json!(0);
    list["is_mutual"] = json!(0);
    list["list_ver"] = json!(version);
    let library = library_page(vec![list]);
    let mut frames = vec![library.clone().into()];
    if rows.is_empty() {
        frames.push(
            reply(json!({"userid":111,"listid":37,"type":0,
            "list_ver":version,"count":0,"info":[]}))
            .into(),
        );
    } else {
        for chunk in rows.chunks(300) {
            frames.push(
                reply(json!({"userid":111,"listid":37,"type":0,
                "list_ver":version,"count":rows.len(),"info":chunk}))
                .into(),
            );
        }
    }
    frames.push(library.into());
    frames
}

fn remove_receipt(count: u64) -> Value {
    json!({"userid":111,"listid":37,"list_ver":2,"pre_list_ver":1,"count":count})
}

#[tokio::test]
async fn standard_remove_provider_confirms_single_and_multiple_occurrences_for_both_owners() {
    for caller in [false, true] {
        for batch in [false, true] {
            let mut before = vec![row(70, 800, 0), row(81, 900, 1)];
            if batch {
                before.extend([row(95, 900, 2), row(99, 901, 3)]);
            }
            let mut frames = start();
            frames.extend(full_snapshot(before, 1));
            frames.push(reply(remove_receipt(1)).into());
            frames.extend(full_snapshot(vec![row(70, 800, 0)], 2));
            let count = frames.len();
            let mut fixture = server(frames).await;
            fixture.provider.client.register_test_device();
            let store = store_account(&mut fixture.provider);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                fixture
                    .provider
                    .caller_scope(&saved.caller().unwrap())
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let mut request = request(if batch { &[900, 901] } else { &[900] }, "A");
            if caller {
                request.account = None;
            }
            let result = provider
                .mutate_playlist_items(REF, PlaylistItemMutationAction::Remove, &request)
                .await
                .unwrap();
            assert_eq!(result.cloud_track_count, Some(1));
            assert_eq!(
                result.extensions["affected_occurrences"],
                if batch { 3 } else { 1 }
            );
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
                assert!(provider.take_response_credential().unwrap().is_none());
            }
            let requests = fixture.requests.await.unwrap();
            assert_eq!(requests.len(), count);
            let writes = requests
                .iter()
                .filter(|r| r.starts_with(PATH))
                .collect::<Vec<_>>();
            assert_eq!(writes.len(), 1);
            let value = body(writes[0]);
            assert_eq!(value["userid"], 111);
            assert_eq!(value["token"], "next");
            assert_eq!(value["list_ver"], 1);
            assert_eq!(value["scene"], "false,2");
            assert_eq!(
                value["data"],
                if batch {
                    json!([{"fileid":81},{"fileid":95},{"fileid":99}])
                } else {
                    json!([{"fileid":81}])
                }
            );
            assert!(
                requests
                    .iter()
                    .all(|r| !r.contains("update_cover") && !r.contains("modify_list"))
            );
        }
    }
}

#[tokio::test]
async fn standard_remove_provider_reads_all_pages_before_and_after_single_dispatch() {
    let before = (0..302)
        .map(|n| row(1000 + n, 2000 + n, n))
        .collect::<Vec<_>>();
    let after = before
        .iter()
        .enumerate()
        .filter(|(n, _)| *n != 0 && *n != 301)
        .map(|(_, v)| v.clone())
        .collect::<Vec<_>>();
    let mut frames = start();
    frames.extend(full_snapshot(before, 1));
    frames.push(reply(remove_receipt(300)).into());
    frames.extend(full_snapshot(after, 2));
    let count = frames.len();
    let mut fixture = server(frames).await;
    fixture.provider.client.register_test_device();
    store_account(&mut fixture.provider);
    let result = fixture
        .provider
        .mutate_playlist_items(
            REF,
            PlaylistItemMutationAction::Remove,
            &request(&[2000, 2301], "A"),
        )
        .await
        .unwrap();
    assert_eq!(result.cloud_track_count, Some(300));
    let requests = fixture.requests.await.unwrap();
    assert_eq!(requests.len(), count);
    let write = requests.iter().find(|r| r.starts_with(PATH)).unwrap();
    assert_eq!(
        body(write)["data"],
        json!([{"fileid":1000},{"fileid":1301}])
    );
}

#[tokio::test]
async fn standard_remove_provider_rejects_partial_or_changed_readback_without_retry() {
    for case in [
        "partial",
        "retained_identity",
        "retained_order",
        "metadata",
        "collaboration",
        "version",
        "count",
    ] {
        let before = vec![
            row(70, 800, 0),
            row(71, 801, 1),
            row(81, 900, 2),
            row(95, 900, 3),
        ];
        let mut after = vec![row(70, 800, 0), row(71, 801, 1)];
        let mut ack = remove_receipt(2);
        let mut version = 2;
        match case {
            "partial" => {
                after.push(row(95, 900, 3));
                ack["count"] = json!(3);
            }
            "retained_identity" => after[0]["fileid"] = json!(69),
            "retained_order" => {
                after[0]["sort"] = json!(1);
                after[1]["sort"] = json!(0);
            }
            "version" => version = 3,
            "count" => ack["count"] = json!(3),
            _ => (),
        }
        let mut frames = start();
        frames.extend(full_snapshot(before, 1));
        frames.push(reply(ack).into());
        let mut after_frames = full_snapshot(after, version);
        if case == "metadata" || case == "collaboration" {
            // Both directory reads agree, but the playlist changed during the write.
            let mut list = list_row(37, 0);
            list["is_def"] = json!(0);
            list["is_mutual"] = json!(if case == "collaboration" { 1 } else { 0 });
            list["list_ver"] = json!(2);
            if case == "metadata" {
                list["name"] = json!("Changed concurrently");
            }
            after_frames[0] = library_page(vec![list.clone()]).into();
            *after_frames.last_mut().unwrap() = library_page(vec![list]).into();
        }
        frames.extend(after_frames);
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        store_account(&mut fixture.provider);
        let error = fixture
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Remove,
                &request(&[900], "A"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!error.retryable);
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), count);
        assert_eq!(requests.iter().filter(|r| r.starts_with(PATH)).count(), 1);
    }
}

#[tokio::test]
async fn standard_remove_provider_rejects_failed_ack_without_retry_or_partial_success() {
    for response in [
        raw(json!({"status":0,"error_code":20010})),
        reply(json!({"userid":222,"listid":37,"list_ver":2,"pre_list_ver":1,"count":0})),
        raw(json!({"status":1,"data":{}})),
    ] {
        let mut frames = start();
        frames.extend(full_snapshot(vec![row(81, 900, 0)], 1));
        frames.push(response.into());
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        let store = store_account(&mut fixture.provider);
        let other = read(&store, "B");
        let error = fixture
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Remove,
                &request(&[900], "A"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert_eq!(error.details["write_requests_dispatched"], 1);
        assert!(!error.retryable);
        assert_eq!(read(&store, "B"), other);
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), count);
        assert_eq!(requests.iter().filter(|r| r.starts_with(PATH)).count(), 1);
    }
}

#[tokio::test]
async fn standard_remove_provider_checks_relogin_on_success_and_failure_ack() {
    for caller in [false, true] {
        for failed in [false, true] {
            let mut frames = start();
            frames.extend(full_snapshot(vec![row(81, 900, 0)], 1));
            let (last, resume) = paused(if failed {
                raw(json!({"status":0,"error_code":20010}))
            } else {
                reply(remove_receipt(0))
            });
            frames.push(last);
            let count = frames.len();
            let mut fixture = server(frames).await;
            fixture.provider.client.register_test_device();
            let store = store_account(&mut fixture.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                fixture
                    .provider
                    .caller_scope(&saved.caller().unwrap())
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let p = provider.clone();
            let task = tokio::spawn(async move {
                let mut value = request(&[900], "A");
                if caller {
                    value.account = None;
                }
                p.mutate_playlist_items(REF, PlaylistItemMutationAction::Remove, &value)
                    .await
            });
            for _ in 0..count {
                fixture.seen.recv().await.unwrap();
            }
            let replacement = credential("111", "replacement-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                    Some(replacement.clone());
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            assert!(error.take_caller_credential_update().is_none());
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
            assert_eq!(fixture.requests.await.unwrap().len(), count);
        }
    }
}

#[tokio::test]
async fn standard_remove_provider_bounds_occurrences_before_write_and_keeps_noop() {
    for case in ["overflow", "budget", "noop"] {
        let rows = match case {
            "overflow" => vec![row(2147483648, 900, 0)],
            "budget" => (1..=301).map(|n| row(n, 900, n)).collect(),
            _ => vec![row(70, 800, 0)],
        };
        let mut frames = start();
        frames.extend(full_snapshot(rows, 1));
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        store_account(&mut fixture.provider);
        let result = fixture
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Remove,
                &request(&[900], "A"),
            )
            .await;
        if case == "noop" {
            let result = result.unwrap();
            assert_eq!(result.extensions["changed"], false);
            assert_eq!(result.extensions["write_requests_dispatched"], 0);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidRequest);
            assert!(error.details.get("write_outcome").is_none());
        }
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), count);
        assert!(requests.iter().all(|r| !r.starts_with(PATH)));
    }
}

#[tokio::test]
async fn standard_remove_provider_rejects_unknown_or_collaborative_targets_before_write() {
    for mutual in [None, Some(1)] {
        let mut selected = list_row(37, 0);
        selected["is_def"] = json!(0);
        selected["list_ver"] = json!(1);
        if let Some(value) = mutual {
            selected["is_mutual"] = json!(value);
        }
        let mut snapshot = full_snapshot(vec![row(81, 900, 0)], 1);
        snapshot[0] = library_page(vec![selected.clone()]).into();
        *snapshot.last_mut().unwrap() = library_page(vec![selected]).into();
        let mut frames = start();
        frames.extend(snapshot);
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        store_account(&mut fixture.provider);
        let error = fixture
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Remove,
                &request(&[900], "A"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
        assert!(error.details.get("write_outcome").is_none());
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), count);
        assert!(requests.iter().all(|r| !r.starts_with(PATH)));
    }
}
