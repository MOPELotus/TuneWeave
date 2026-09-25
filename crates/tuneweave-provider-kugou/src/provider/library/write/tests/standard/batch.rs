use super::*;

fn batch_catalogue() -> Vec<Frame> {
    let mut frames = standard_catalogue(audio());
    frames.push(
        reply(
            json!([{"__status":1,"base":{"album_audio_id":901,"audio_id":1901,
        "songname":"Song 901","author_name":"Artist","album_id":43,"album_name":"Other"}}]),
        )
        .into(),
    );
    let mut second = audio();
    second["audio_id"] = json!(1901);
    second["audio_name"] = json!("Artist - Song 901");
    second["hash"] = json!("b".repeat(32));
    frames.push(reply(json!([second])).into());
    frames
}

fn batch_receipt() -> Value {
    let mut value = standard_receipt();
    let mut second = value["info"][0].clone();
    second["fileid"] = json!(100);
    second["mixsongid"] = json!(901);
    second["sort"] = json!(2);
    second["hash"] = json!("b".repeat(32));
    second["album_id"] = json!("43");
    value["info"] = json!([second, value["info"][0]]);
    value["count"] = json!(3);
    value
}

fn batch_after() -> Vec<Value> {
    let mut second = row(100, 901, 2);
    second["hash"] = json!("b".repeat(32));
    second["album_id"] = json!(43);
    second["album_name"] = json!("Other");
    vec![row(70, 800, 0), added_row(), second]
}

fn before_write() -> Vec<Frame> {
    let mut frames = start();
    frames.extend(ordinary_snapshot(vec![row(70, 800, 0)], 1));
    frames.extend(batch_catalogue());
    frames.extend(ordinary_snapshot(vec![row(70, 800, 0)], 1));
    frames
}

fn batch_request(caller: bool) -> PlaylistItemMutationRequest {
    // Existing 800 remains untouched and is excluded from the network batch.
    let mut value = request(&[800, 900, 901], "A");
    if caller {
        value.account = None;
    }
    value
}

#[tokio::test]
async fn standard_add_batch_provider_confirms_one_ordered_batch_for_both_owners() {
    for caller in [false, true] {
        let mut frames = before_write();
        frames.push(reply(batch_receipt()).into());
        frames.extend(ordinary_snapshot(batch_after(), 2));
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
        let result = provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &batch_request(caller))
            .await
            .unwrap();
        assert_eq!(result.cloud_track_count, Some(3));
        assert_eq!(result.extensions["affected_occurrences"], 2);
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
            .filter(|r| r.starts_with("POST /cloudlist.service/v6/add_song?"))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 1);
        let value = body(writes[0]);
        assert_eq!(value["list_ver"], 1);
        assert_eq!(value["userid"], "111");
        assert_eq!(value["token"], "next");
        assert_eq!(value["data"].as_array().unwrap().len(), 2);
        assert_eq!(value["data"][0]["mixsongid"], 901);
        assert_eq!(value["data"][0]["sort"], 1);
        assert_eq!(value["data"][1]["mixsongid"], 900);
        assert_eq!(value["data"][1]["sort"], 0);
    }
}

#[tokio::test]
async fn standard_add_batch_provider_never_confirms_or_retries_partial_ack() {
    for missing in [false, true] {
        let mut receipt = batch_receipt();
        if missing {
            receipt["info"].as_array_mut().unwrap().pop();
        } else {
            receipt["info"][0]["code"] = json!(205);
        }
        let mut frames = before_write();
        frames.push(reply(receipt).into());
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        let store = store_account(&mut fixture.provider);
        let other = read(&store, "B");
        let error = fixture
            .provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &batch_request(false))
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert_eq!(error.details["write_requests_dispatched"], 1);
        assert!(!error.retryable);
        assert_eq!(read(&store, "B"), other);
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), count);
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /cloudlist.service/v6/add_song?"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn standard_add_batch_provider_requires_every_readback_identity_and_input_order() {
    for case in ["file", "hash", "album", "input_order"] {
        let mut after = batch_after();
        let mut receipt = batch_receipt();
        match case {
            "file" => after[2]["fileid"] = json!(101),
            "hash" => after[2]["hash"] = json!("c".repeat(32)),
            "album" => after[2]["album_id"] = json!(44),
            _ => {
                // Individually valid ACK/rows, but the new songs swap order.
                receipt["info"][0]["sort"] = json!(1);
                receipt["info"][1]["sort"] = json!(2);
                after[1]["sort"] = json!(2);
                after[2]["sort"] = json!(1);
            }
        }
        let mut frames = before_write();
        frames.push(reply(receipt).into());
        frames.extend(ordinary_snapshot(after, 2));
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        store_account(&mut fixture.provider);
        let error = fixture
            .provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &batch_request(false))
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed", "{case}");
        assert_eq!(error.details["write_requests_dispatched"], 1);
        assert!(!error.retryable);
        assert_eq!(fixture.requests.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn standard_add_batch_provider_checks_relogin_on_success_and_failure_ack() {
    for caller in [false, true] {
        for failed in [false, true] {
            let mut frames = before_write();
            let (last, resume) = paused(if failed {
                raw(json!({"status":0,"error_code":20010}))
            } else {
                reply(batch_receipt())
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
                p.mutate_playlist_items(
                    REF,
                    PlaylistItemMutationAction::Add,
                    &batch_request(caller),
                )
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
