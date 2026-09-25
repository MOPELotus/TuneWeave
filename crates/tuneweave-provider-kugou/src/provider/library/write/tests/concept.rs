use super::*;
use crate::provider::session::tests::Store;

mod add;
mod favorite;

fn concept_store(provider: &mut KugouProvider) -> Arc<Store> {
    let store = store_account(provider);
    let mut session = read(&store, "A").native().session.clone();
    session.client = KugouLoginClient::Concept;
    store
        .put(
            &KugouCredential::verified(session)
                .unwrap()
                .stored("A")
                .unwrap(),
        )
        .unwrap();
    store
}
fn list(version: u64) -> Value {
    let mut value = list_row(37, 0);
    value["list_ver"] = json!(version);
    value["is_def"] = json!(0);
    value
}
fn ordinary_snapshot(rows: Vec<Value>, version: u64, selected: Value) -> Vec<Frame> {
    let library = library_page(vec![selected]);
    let mut frames = vec![library.clone().into()];
    if rows.is_empty() {
        frames.push(
            reply(
                json!({"userid":111,"listid":37,"type":0,"list_ver":version,"count":0,"info":[]}),
            )
            .into(),
        );
    } else {
        for chunk in rows.chunks(300) {
            frames.push(reply(json!({"userid":111,"listid":37,"type":0,"list_ver":version,"count":rows.len(),"info":chunk})).into());
        }
    }
    frames.push(library.into());
    frames
}
fn remove_request(ids: &[u64], caller: bool) -> PlaylistItemMutationRequest {
    let mut r = request(ids, "A");
    if caller {
        r.account = None;
    }
    r
}

#[tokio::test]
async fn concept_occurrence_remove_provider_reads_all_pages_and_preserves_other_occurrences() {
    for caller in [false, true] {
        let after = (0..300)
            .map(|i| row(i + 100, 1000 + i, i))
            .collect::<Vec<_>>();
        let mut before = after.clone();
        before.push(row(777, 900, 300));
        let mut frames = start();
        frames.extend(ordinary_snapshot(before, 7, list(7)));
        frames.push(acknowledgement(7, 8, 300));
        frames.extend(ordinary_snapshot(after, 8, list(8)));
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
                PlaylistItemMutationAction::Remove,
                &remove_request(&[900], caller),
            )
            .await
            .unwrap();
        assert_eq!(result.cloud_track_count, Some(300));
        assert_eq!(result.extensions["affected_occurrences"], 1);
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(p.take_response_credential().unwrap().is_some());
        }
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 10);
        let writes = all
            .iter()
            .filter(|r| r.starts_with("POST /cloudlist.service/v4/delete_songs?"))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            body(writes[0]),
            json!({"userid":111,"token":"next","listid":37,"type":0,"list_ver":7,"data":[{"fileid":777}]})
        );
        assert!(all.iter().all(|r| !r.starts_with("POST /v4/delete_songs?")));
    }
}

#[tokio::test]
async fn concept_occurrence_remove_provider_rejects_batches_duplicates_systems_and_foreign_targets()
{
    for caller in [false, true] {
        let mut f = server(vec![]).await;
        let store = concept_store(&mut f.provider);
        let saved = read(&store, "A");
        let p = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        assert_eq!(
            p.mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Remove,
                &remove_request(&[900, 901], caller)
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::CapabilityNotSupported
        );
        for id in ["cloudlist:111:1:37", "cloudlist:222:0:37"] {
            assert!(
                p.mutate_playlist_items(
                    id,
                    PlaylistItemMutationAction::Remove,
                    &remove_request(&[900], caller)
                )
                .await
                .is_err()
            );
        }
        assert_eq!(read(&store, "A"), saved);
        assert!(f.requests.await.unwrap().is_empty());
    }
    for case in ["duplicate", "system", "missing_class"] {
        let mut rows = vec![row(81, 900, 0)];
        let mut selected = list(7);
        match case {
            "duplicate" => rows.push(row(95, 900, 1)),
            "system" => selected["is_def"] = json!(2),
            _ => {
                selected.as_object_mut().unwrap().remove("is_def");
            }
        }
        let mut frames = start();
        frames.extend(ordinary_snapshot(rows, 7, selected));
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let e = f
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Remove,
                &remove_request(&[900], false),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::CapabilityNotSupported, "{case}");
        assert!(e.details.get("write_outcome").is_none());
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn concept_occurrence_remove_provider_absent_song_is_a_complete_read_noop() {
    let mut frames = start();
    frames.extend(ordinary_snapshot(vec![row(81, 800, 0)], 7, list(7)));
    let mut f = server(frames).await;
    concept_store(&mut f.provider);
    let result = f
        .provider
        .mutate_playlist_items(
            REF,
            PlaylistItemMutationAction::Remove,
            &remove_request(&[900], false),
        )
        .await
        .unwrap();
    assert_eq!(result.extensions["changed"], false);
    assert_eq!(result.extensions["write_requests_dispatched"], 0);
    assert_eq!(result.cloud_track_count, Some(1));
    assert_eq!(f.requests.await.unwrap().len(), 5);
}

#[tokio::test]
async fn concept_occurrence_remove_provider_requires_exact_delta_order_and_ack_version() {
    for case in [
        "retained",
        "other_fileid",
        "other_order",
        "metadata",
        "ack_count",
        "ack_version",
    ] {
        let before = vec![row(70, 800, 0), row(81, 900, 1), row(95, 801, 2)];
        let mut after = vec![row(70, 800, 0), row(95, 801, 2)];
        let mut selected = list(8);
        let mut count = 2;
        let mut version = 8;
        match case {
            "retained" => after.insert(1, row(81, 900, 1)),
            "other_fileid" => after[0]["fileid"] = json!(71),
            "other_order" => after[0]["sort"] = json!(3),
            "metadata" => selected["name"] = json!("Unrequested rename"),
            "ack_count" => count = 3,
            _ => version = 9,
        }
        let mut frames = start();
        frames.extend(ordinary_snapshot(before, 7, list(7)));
        frames.push(acknowledgement(7, version, count));
        frames.extend(ordinary_snapshot(after, 8, selected));
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let e = f
            .provider
            .mutate_playlist_items(
                REF,
                PlaylistItemMutationAction::Remove,
                &remove_request(&[900], false),
            )
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 9);
    }
}

#[tokio::test]
async fn concept_occurrence_remove_provider_discards_late_write_and_readback_results() {
    for caller in [false, true] {
        for readback in [false, true] {
            for failed in [false, true] {
                let mut frames = start();
                frames.extend(ordinary_snapshot(
                    vec![row(70, 800, 0), row(81, 900, 1)],
                    7,
                    list(7),
                ));
                if readback {
                    frames.push(acknowledgement(7, 8, 1));
                }
                let response = if failed {
                    raw(json!({"status":0,"error_code":20017}))
                } else if readback {
                    library_page(vec![list(8)])
                } else {
                    reply(json!({"userid":111,"listid":37,"list_ver":8,"pre_list_ver":7,"count":1}))
                };
                let (frame, resume) = paused(response);
                frames.push(frame);
                let count = frames.len();
                let mut f = server(frames).await;
                let store = concept_store(&mut f.provider);
                let saved = read(&store, "A");
                let p = if caller {
                    f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
                } else {
                    f.provider.clone()
                };
                let worker = p.clone();
                let task = tokio::spawn(async move {
                    worker
                        .mutate_playlist_items(
                            REF,
                            PlaylistItemMutationAction::Remove,
                            &remove_request(&[900], caller),
                        )
                        .await
                });
                for _ in 0..count {
                    f.seen.recv().await.unwrap();
                }
                let mut session = saved.native().session.clone();
                session.token = "new-login".into();
                let replacement = KugouCredential::verified(session).unwrap();
                if caller {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() =
                        Some(replacement.clone());
                } else {
                    store.put(&replacement.stored("A").unwrap()).unwrap();
                }
                resume.send(()).unwrap();
                let mut e = task.await.unwrap().unwrap_err();
                assert_eq!(e.code, ErrorCode::Conflict);
                assert_eq!(e.details["write_outcome"], "unconfirmed");
                assert!(e.take_caller_credential_update().is_none());
                assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
                assert_eq!(f.requests.await.unwrap().len(), count);
            }
        }
    }
}
