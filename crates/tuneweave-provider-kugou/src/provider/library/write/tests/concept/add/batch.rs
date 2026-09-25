use super::*;

fn batch_song(id: u64) -> Value {
    let mut value = song(id);
    value["audio_info"]["hash"] = json!(format!("{id:032x}"));
    value
}
fn lookup(id: u64) -> Vec<Frame> {
    vec![
        reply(json!([{"__status":1,"base":{"album_audio_id":id,"album_id":42}}])).into(),
        album(vec![batch_song(id)], 1).into(),
    ]
}
fn batch_ack(ids: &[u64], count: usize) -> Value {
    let info = ids
        .iter()
        .enumerate()
        .rev()
        .map(|(index, id)| {
            json!({
                "fileid":90+index,"sort":index,"name":"Artist - Song.mp3",
                "hash":format!("{id:032x}"),"album_id":"42","mixsongid":id
            })
        })
        .collect::<Vec<_>>();
    json!({"userid":111,"listid":37,"pre_list_ver":7,"list_ver":8,"count":count,"info":info})
}
fn batch_after(ids: &[u64], retained: &[Value]) -> Vec<Value> {
    let mut result = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let mut v = row(90 + index as u64, *id, index as u64);
            v["album_id"] = json!(42);
            v["hash"] = json!(format!("{id:032x}"));
            v
        })
        .collect::<Vec<_>>();
    result.extend(retained.iter().cloned().map(|mut v| {
        v["sort"] = json!(v["sort"].as_u64().unwrap() + ids.len() as u64);
        v
    }));
    result
}
fn before_write(before: &[Value], ids: &[u64]) -> Vec<Frame> {
    let mut frames = start();
    frames.extend(ordinary_snapshot(before.to_vec(), 7, list(7)));
    for id in ids {
        frames.extend(lookup(*id));
    }
    frames.extend(ordinary_snapshot(before.to_vec(), 7, list(7)));
    frames
}

#[tokio::test]
async fn concept_batch_add_provider_confirms_complete_ordered_delta_for_both_owners() {
    for caller in [false, true] {
        let before = (0..301)
            .map(|n| row(1000 + n, 2000 + n, n))
            .collect::<Vec<_>>();
        let mut frames = before_write(&before, &[900, 901]);
        frames.push(reply(batch_ack(&[900, 901], 303)).into());
        frames.extend(ordinary_snapshot(
            batch_after(&[900, 901], &before),
            8,
            list(8),
        ));
        let count = frames.len();
        let mut f = server(frames).await;
        let store = concept_store(&mut f.provider);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let p = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let result = p
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Add,
                &remove_request(&[900, 901], caller),
            )
            .await
            .unwrap();
        assert_eq!(result.cloud_track_count, Some(303));
        assert_eq!(result.extensions["affected_occurrences"], 2);
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(p.take_response_credential().unwrap().is_some());
        }
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), count);
        let writes = requests
            .iter()
            .filter(|r| r.starts_with("POST /cloudlist.service/v4/add_song?"))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 1);
        assert_eq!(body(writes[0])["data"][0]["mixsongid"], 901);
        assert_eq!(body(writes[0])["data"][1]["mixsongid"], 900);
        for request in requests
            .iter()
            .filter(|r| r.starts_with("POST /kmr/v1/album_songlist?"))
        {
            assert!(!request.contains("next"));
            assert!(!request.contains("userid"));
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
        }
    }
}

#[tokio::test]
async fn concept_batch_add_provider_existing_unique_inputs_are_read_only_or_not_readded() {
    for all_present in [false, true] {
        let before = vec![row(70, 900, 0), row(71, 800, 1)];
        let ids = if all_present {
            vec![900, 800]
        } else {
            vec![900, 901]
        };
        let mut frames = if all_present {
            let mut frames = start();
            frames.extend(ordinary_snapshot(before.clone(), 7, list(7)));
            frames
        } else {
            before_write(&before, &[901])
        };
        if !all_present {
            frames.push(reply(batch_ack(&[901], 3)).into());
            frames.extend(ordinary_snapshot(batch_after(&[901], &before), 8, list(8)));
        }
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let result = f
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Add,
                &remove_request(&ids, false),
            )
            .await
            .unwrap();
        assert_eq!(
            result.extensions["write_requests_dispatched"],
            usize::from(!all_present)
        );
        let requests = f.requests.await.unwrap();
        let writes = requests
            .iter()
            .filter(|r| r.starts_with("POST /cloudlist.service/v4/add_song?"))
            .collect::<Vec<_>>();
        if all_present {
            assert!(writes.is_empty());
        } else {
            assert_eq!(body(writes[0])["data"].as_array().unwrap().len(), 1);
            assert_eq!(body(writes[0])["data"][0]["mixsongid"], 901);
        }
    }
}

#[tokio::test]
async fn concept_batch_add_provider_partial_receipts_and_bad_readback_stay_unconfirmed() {
    for case in [
        "partial",
        "capacity",
        "duplicate_file",
        "wrong_order",
        "count",
        "changed_existing",
    ] {
        let before = vec![row(70, 800, 0), row(71, 801, 1)];
        let mut frames = before_write(&before, &[900, 901]);
        let mut ack = batch_ack(&[900, 901], 4);
        match case {
            "partial" => {
                ack["info"].as_array_mut().unwrap().pop();
            }
            "capacity" => ack["info"][0]["code"] = json!(205),
            "duplicate_file" => ack["info"][0]["fileid"] = json!(90),
            "count" => ack["count"] = json!(3),
            _ => {}
        }
        frames.push(reply(ack).into());
        if matches!(case, "wrong_order" | "count" | "changed_existing") {
            let mut rows = batch_after(&[900, 901], &before);
            if case == "wrong_order" {
                rows.swap(0, 1);
                rows[0]["sort"] = json!(0);
                rows[1]["sort"] = json!(1);
            }
            if case == "changed_existing" {
                rows[2]["hash"] = json!("b".repeat(32));
            }
            frames.extend(ordinary_snapshot(rows, 8, list(8)));
        }
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let error = f
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Add,
                &remove_request(&[900, 901], false),
            )
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!error.retryable);
        assert_eq!(
            f.requests
                .await
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST /cloudlist.service/v4/add_song?"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn concept_batch_add_provider_rejects_ambiguous_hash_and_shared_byte_budget_before_write() {
    for oversized in [false, true] {
        let ids = if oversized {
            vec![900, 901, 902, 903, 904]
        } else {
            vec![900, 901]
        };
        let mut frames = start();
        frames.extend(ordinary_snapshot(vec![row(70, 800, 0)], 7, list(7)));
        for id in &ids {
            frames.push(
                reply(json!([{"__status":1,"base":{"album_audio_id":id,"album_id":42}}])).into(),
            );
            let mut value = batch_song(*id);
            if oversized {
                value["padding"] = json!("x".repeat(850_000));
            } else {
                value["audio_info"]["hash"] = json!("a".repeat(32));
            }
            frames.push(album(vec![value], 1).into());
        }
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let error = f
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Add,
                &remove_request(&ids, false),
            )
            .await
            .unwrap_err();
        assert!(error.details.get("write_outcome").is_none());
        assert!(
            !f.requests
                .await
                .unwrap()
                .iter()
                .any(|r| r.starts_with("POST /cloudlist.service/v4/add_song?"))
        );
    }
}

#[tokio::test]
async fn concept_batch_add_provider_relogin_discards_late_write_ack_or_failure() {
    for caller in [false, true] {
        for failed in [false, true] {
            let mut frames = before_write(&[row(70, 800, 0)], &[900, 901]);
            let (frame, resume) = paused(if failed {
                raw(json!({"status":0,"error_code":20017}))
            } else {
                reply(batch_ack(&[900, 901], 3))
            });
            frames.push(frame);
            let count = frames.len();
            let mut f = server(frames).await;
            let store = concept_store(&mut f.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let p = provider.clone();
            let task = tokio::spawn(async move {
                p.mutate_playlist_items(
                    REF,
                    PlaylistItemMutationAction::Add,
                    &remove_request(&[900, 901], caller),
                )
                .await
            });
            for _ in 0..count {
                f.seen.recv().await.unwrap();
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
            assert!(!error.retryable);
            assert!(error.take_caller_credential_update().is_none());
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
            assert_eq!(f.requests.await.unwrap().len(), count);
        }
    }
}
