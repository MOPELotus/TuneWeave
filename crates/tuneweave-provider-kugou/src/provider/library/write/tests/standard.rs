use super::*;

mod batch;
mod remove;

fn ordinary_snapshot(rows: Vec<Value>, version: u64) -> Vec<Frame> {
    let mut selected = list_row(37, 0);
    selected["is_def"] = json!(0);
    selected["list_ver"] = json!(version);
    let library = library_page(vec![selected]);
    vec![
        library.clone().into(),
        reply(json!({"userid":111,"listid":37,"type":0,"list_ver":version,
            "count":rows.len(),"info":rows}))
        .into(),
        library.into(),
    ]
}

fn audio() -> Value {
    json!({"audio_id":1900,"audio_name":"Artist - Song 900",
        "hash":"abcdef0123456789abcdef0123456789","filesize":120000,
        "bitrate":128000,"timelength":216789})
}

fn standard_catalogue(audio: Value) -> Vec<Frame> {
    vec![
        reply(
            json!([{"__status":1,"base":{"album_audio_id":900,"audio_id":1900,
            "songname":"Song 900","author_name":"Artist","album_id":42,"album_name":"Album"}}]),
        )
        .into(),
        reply(json!([audio])).into(),
    ]
}

fn add_request(caller: bool) -> PlaylistItemMutationRequest {
    let mut request = request(&[900], "A");
    if caller {
        request.account = None;
    }
    request
}

fn standard_receipt() -> Value {
    json!({"userid":111,"listid":37,"list_ver":2,"pre_list_ver":1,"count":2,
        "info":[{"fileid":99,"mixsongid":900,"sort":1,"name":"Artist - Song 900.mp3",
        "hash":"abcdef0123456789abcdef0123456789","album_id":"42","code":1}]})
}

fn added_row() -> Value {
    let mut value = row(99, 900, 1);
    value["album_id"] = json!(42);
    value["album_name"] = json!("Album");
    value
}

#[tokio::test]
async fn standard_add_provider_maps_ordinary_items_for_both_owners() {
    for client in [KugouLoginClient::Standard] {
        for caller in [false, true] {
            let before = vec![row(70, 800, 0)];
            let mut frames = start();
            frames.extend(ordinary_snapshot(before.clone(), 1));
            frames.extend(standard_catalogue(audio()));
            frames.extend(ordinary_snapshot(before, 1));
            frames.push(reply(standard_receipt()).into());
            frames.extend(ordinary_snapshot(vec![row(70, 800, 0), added_row()], 2));
            let count = frames.len();
            let mut fixture = server(frames).await;
            fixture.provider.client.register_test_device();
            let store = store_account(&mut fixture.provider);
            if client == KugouLoginClient::Concept {
                let mut session = read(&store, "A").native().session.clone();
                session.client = client;
                store
                    .put(
                        &KugouCredential::verified(session)
                            .unwrap()
                            .stored("A")
                            .unwrap(),
                    )
                    .unwrap();
            }
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
                .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(caller))
                .await
                .unwrap();
            assert_eq!(result.cloud_track_count, Some(2));
            assert_eq!(result.extensions["affected_occurrences"], 1);
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
            let path = if client == KugouLoginClient::Standard {
                "POST /cloudlist.service/v6/add_song?"
            } else {
                "POST /cloudlist.service/v4/add_song?"
            };
            let writes = requests
                .iter()
                .filter(|request| request.starts_with(path))
                .collect::<Vec<_>>();
            assert_eq!(writes.len(), 1);
            let body = body(writes[0]);
            assert_eq!(body["userid"], "111");
            assert_eq!(body["token"], "next");
            assert_eq!(body["listid"], 37);
            assert_eq!(body["list_ver"], 1);
            assert_eq!(body["mode"], 1);
            assert_eq!(body["allow_part_fail"], 1);
            assert_eq!(body["data"][0]["mixsongid"], 900);
            if client == KugouLoginClient::Standard {
                assert_eq!(body["data"][0]["name"], "Artist - Song 900.mp3");
                assert_eq!(body["data"][0]["timelen"], 216789);
                assert_eq!(body["data"][0]["size"], 120000);
                assert_eq!(body["data"][0]["bitrate"], 128);
                assert_eq!(body["data"][0]["album_id"], "42");
            } else {
                assert_eq!(body["data"][0]["name"], "Artist - Song 900");
                assert_eq!(body["data"][0]["timelen"], 0);
                assert_eq!(body["data"][0]["bitrate"], 0);
                assert_eq!(body["data"][0]["album_id"], 42);
            }
            for request in requests.iter().filter(|r| {
                r.starts_with("POST /kmr/v2/audio?") || r.starts_with("POST /v1/audio/audio?")
            }) {
                assert!(!request.contains("token="));
                assert!(!request.contains("next"));
            }
        }
    }
}

#[tokio::test]
async fn standard_add_provider_rejects_missing_units_and_wrong_audio_identity_before_write() {
    for case in [
        "missing_duration",
        "large_size",
        "missing_bitrate",
        "wrong_audio",
    ] {
        let mut value = audio();
        match case {
            "missing_duration" => value.as_object_mut().unwrap().remove("timelength"),
            "large_size" => value
                .as_object_mut()
                .unwrap()
                .insert("filesize".into(), json!(i32::MAX as u64 + 1)),
            "missing_bitrate" => value.as_object_mut().unwrap().remove("bitrate"),
            _ => value
                .as_object_mut()
                .unwrap()
                .insert("audio_id".into(), json!(999)),
        };
        let mut frames = start();
        frames.extend(ordinary_snapshot(vec![row(70, 800, 0)], 1));
        frames.extend(standard_catalogue(value));
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        let store = store_account(&mut fixture.provider);
        let other = read(&store, "B");
        let error = fixture
            .provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(false))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError, "{case}");
        assert!(error.details.get("write_outcome").is_none());
        assert_eq!(read(&store, "B"), other);
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), 7);
        assert!(
            !requests
                .iter()
                .any(|r| r.starts_with("POST /cloudlist.service/v6/add_song?"))
        );
    }
}

#[tokio::test]
async fn standard_add_provider_requires_exact_readback_after_one_write() {
    for wrong_ack in [false, true] {
        let before = vec![row(70, 800, 0)];
        let mut frames = start();
        frames.extend(ordinary_snapshot(before.clone(), 1));
        frames.extend(standard_catalogue(audio()));
        frames.extend(ordinary_snapshot(before.clone(), 1));
        if wrong_ack {
            let mut receipt = standard_receipt();
            receipt["userid"] = json!(222);
            frames.push(reply(receipt).into());
        } else {
            frames.push(reply(standard_receipt()).into());
            frames.extend(ordinary_snapshot(before, 2));
        }
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        store_account(&mut fixture.provider);
        let error = fixture
            .provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(false))
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert!(!error.retryable);
        assert_eq!(
            fixture
                .requests
                .await
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST /cloudlist.service/v6/add_song?"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn standard_add_provider_relogin_rejects_late_ack_without_retry_or_readback() {
    for caller in [false, true] {
        for failed in [false, true] {
            let before = vec![row(70, 800, 0)];
            let mut frames = start();
            frames.extend(ordinary_snapshot(before.clone(), 1));
            frames.extend(standard_catalogue(audio()));
            frames.extend(ordinary_snapshot(before, 1));
            let (last, resume) = paused(if failed {
                raw(json!({"status":0,"error_code":20010}))
            } else {
                reply(standard_receipt())
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
                p.mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(caller))
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
}

#[tokio::test]
async fn standard_add_single_provider_rejects_summary_partial_or_replacement_ack() {
    for case in [
        "missing_info",
        "item_failed",
        "wrong_mix",
        "stale_version",
        "replacement",
    ] {
        let mut receipt = standard_receipt();
        match case {
            "missing_info" => {
                receipt.as_object_mut().unwrap().remove("info");
            }
            "item_failed" => receipt["info"][0]["code"] = json!(0),
            "wrong_mix" => receipt["info"][0]["mixsongid"] = json!(901),
            "stale_version" => receipt["pre_list_ver"] = json!(0),
            _ => receipt["del_fileids"] = json!([70]),
        }
        let before = vec![row(70, 800, 0)];
        let mut frames = start();
        frames.extend(ordinary_snapshot(before.clone(), 1));
        frames.extend(standard_catalogue(audio()));
        frames.extend(ordinary_snapshot(before, 1));
        frames.push(reply(receipt).into());
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        let store = store_account(&mut fixture.provider);
        let other = read(&store, "B");
        let error = fixture
            .provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(false))
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!error.retryable);
        assert_eq!(read(&store, "B"), other);
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), count, "{case}");
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
async fn standard_add_single_provider_checks_ack_file_sort_hash_and_album_against_readback() {
    for field in ["fileid", "sort", "hash", "album_id"] {
        let mut item = added_row();
        item[field] = match field {
            "fileid" => json!(100),
            "sort" => json!(2),
            "hash" => json!("b".repeat(32)),
            _ => json!(43),
        };
        let before = vec![row(70, 800, 0)];
        let mut frames = start();
        frames.extend(ordinary_snapshot(before.clone(), 1));
        frames.extend(standard_catalogue(audio()));
        frames.extend(ordinary_snapshot(before, 1));
        frames.push(reply(standard_receipt()).into());
        frames.extend(ordinary_snapshot(vec![row(70, 800, 0), item], 2));
        let count = frames.len();
        let mut fixture = server(frames).await;
        fixture.provider.client.register_test_device();
        store_account(&mut fixture.provider);
        let error = fixture
            .provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(false))
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed", "{field}");
        assert!(!error.retryable);
        assert_eq!(error.details["write_requests_dispatched"], 1);
        assert_eq!(fixture.requests.await.unwrap().len(), count);
    }
}
