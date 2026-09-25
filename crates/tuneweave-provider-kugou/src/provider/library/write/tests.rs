use super::super::tests::{page as library_page, row as list_row, store_account};
use super::*;
use crate::provider::session::tests::{
    Frame, credential, exchange, paused, profile, raw, read, reply, server,
};

mod concept;
mod standard;

const REF: &str = "cloudlist:111:0:37";
fn row(file: u64, id: u64, sort: u64) -> Value {
    json!({"fileid":file,"mixsongid":id,"sort":sort,"name":format!("Song {id}"),
        "hash":"abcdef0123456789abcdef0123456789","timelen":123000})
}
fn snapshot(rows: Vec<Value>, version: u64) -> Vec<Frame> {
    let mut list = list_row(37, 0);
    list["is_def"] = json!(2);
    list["list_ver"] = json!(version);
    let library = library_page(vec![list]);
    let mut frames = vec![library.clone().into()];
    if rows.is_empty() {
        frames.push(reply(json!({"list_ver":version,"count":0,"info":[]})).into());
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
fn start() -> Vec<Frame> {
    vec![exchange("111", "next").into(), profile("111").into()]
}
fn acknowledgement(before: u64, after: u64, count: u64) -> Frame {
    reply(json!({"userid":111,"listid":37,"type":0,"pre_list_ver":before,"list_ver":after,"count":count})).into()
}
fn catalogue(id: u64) -> Vec<Frame> {
    vec![reply(json!([{"__status":1,"base":{"album_audio_id":id,"audio_id":id+1000,
        "songname":format!("Song {id}"),"author_name":"Artist"}}])).into(),
        reply(json!([{"audio_id":id+1000,"audio_name":format!("Artist - Song {id}"),
            "hash":"abcdef0123456789abcdef0123456789","filesize":120000,"bitrate":128,"timelength":123000}])).into()]
}
fn request(ids: &[u64], account: &str) -> PlaylistItemMutationRequest {
    PlaylistItemMutationRequest {
        item_refs: ids
            .iter()
            .map(|id| ResourceRef::new(Platform::Kugou, id.to_string()).unwrap())
            .collect(),
        kind: PlaylistItemKind::Track,
        account: Some(account.into()),
    }
}
fn body(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn likes_adds_from_verified_catalogue_and_removes_every_matching_occurrence_for_each_owner() {
    for caller in [false, true] {
        for add in [false, true] {
            let before = if add {
                vec![row(70, 800, 0)]
            } else {
                vec![row(70, 800, 0), row(81, 900, 1), row(95, 900, 2)]
            };
            let after = if add {
                vec![row(70, 800, 0), row(99, 900, 1)]
            } else {
                vec![row(70, 800, 0)]
            };
            let mut frames = start();
            frames.extend(snapshot(before.clone(), 1));
            if add {
                frames.extend(catalogue(900));
                frames.extend(snapshot(before, 1));
            }
            frames.push(acknowledgement(1, 2, after.len() as u64));
            frames.extend(snapshot(after, 2));
            let n = frames.len();
            let mut f = server(frames).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let result = provider
                .set_track_subscription("900", add, if caller { None } else { Some("A") })
                .await
                .unwrap();
            assert_eq!(result.subscribed, add);
            assert_eq!(result.resource_ref.id(), "900");
            assert_eq!(
                result.extensions["affected_occurrences"],
                if add { 1 } else { 2 }
            );
            assert_eq!(result.extensions["changed"], true);
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
                assert!(provider.take_response_credential().unwrap().is_none());
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), n);
            let writes = requests
                .iter()
                .filter(|r| {
                    r.starts_with("POST /cloudlist.service/v6/add_song?")
                        || r.starts_with("POST /v4/delete_songs?")
                })
                .collect::<Vec<_>>();
            assert_eq!(writes.len(), 1);
            let b = body(writes[0]);
            assert_eq!(b["listid"], 37);
            assert_eq!(b["userid"], 111);
            assert_eq!(b["token"], "next");
            if add {
                assert_eq!(b["data"][0]["mixsongid"], 900);
                assert_eq!(b["data"][0]["name"], "Artist - Song 900");
                for r in requests.iter().filter(|r| {
                    r.starts_with("POST /kmr/v2/audio?") || r.starts_with("POST /v1/audio/audio?")
                }) {
                    assert!(!r.contains("token="));
                    assert!(!r.contains("next"));
                }
            } else {
                assert_eq!(b["data"], json!([{"fileid":81},{"fileid":95}]));
            }
        }
    }
}

#[tokio::test]
async fn already_liked_or_absent_tracks_do_not_dispatch_writes_or_lookup_catalogue() {
    for add in [false, true] {
        let rows = if add {
            vec![row(81, 900, 0), row(95, 900, 1)]
        } else {
            vec![row(70, 800, 0)]
        };
        let mut frames = start();
        frames.extend(snapshot(rows, 1));
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let result = f
            .provider
            .set_track_subscription("900", add, Some("A"))
            .await
            .unwrap();
        assert_eq!(result.extensions["changed"], false);
        assert_eq!(result.extensions["write_requests_dispatched"], 0);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn ordinary_mutations_remove_in_bounded_batches_and_preserve_non_target_order() {
    let mut before = vec![row(1, 800, 0)];
    before.extend((0..302).map(|i| row(100 + i, 900, i + 1)));
    before.push(row(2, 801, 303));
    let after = vec![row(1, 800, 0), row(2, 801, 1)];
    let mut frames = start();
    frames.extend(snapshot(before, 1));
    frames.push(acknowledgement(1, 2, 4));
    frames.push(acknowledgement(2, 3, 2));
    frames.extend(snapshot(after, 3));
    let mut f = server(frames).await;
    store_account(&mut f.provider);
    let result = f
        .provider
        .mutate_playlist_items(
            REF,
            PlaylistItemMutationAction::Remove,
            &request(&[900], "A"),
        )
        .await
        .unwrap();
    assert_eq!(result.cloud_track_count, Some(2));
    assert_eq!(result.extensions["affected_occurrences"], 302);
    assert_eq!(result.extensions["write_requests_dispatched"], 2);
    assert!(result.snapshot_id.is_some());
    let requests = f.requests.await.unwrap();
    let writes = requests
        .iter()
        .filter(|r| r.starts_with("POST /v4/delete_songs?"))
        .map(|r| body(r))
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0]["data"].as_array().unwrap().len(), 300);
    assert_eq!(writes[1]["data"].as_array().unwrap().len(), 2);
    assert_eq!(writes[0]["data"][0]["fileid"], 100);
    assert_eq!(writes[1]["data"][1]["fileid"], 401);
    assert!(writes.iter().all(|b| b["list_ver"] == 0));
}

#[tokio::test]
async fn add_batch_skips_existing_tracks_and_retains_requested_result_order() {
    let before = vec![row(1, 800, 0)];
    let after = vec![row(1, 800, 0), row(81, 901, 1), row(95, 900, 2)];
    let mut frames = start();
    frames.extend(snapshot(before.clone(), 1));
    frames.extend(catalogue(901));
    frames.extend(catalogue(900));
    frames.extend(snapshot(before, 1));
    frames.push(acknowledgement(1, 2, 3));
    frames.extend(snapshot(after, 2));
    let mut f = server(frames).await;
    f.provider.client.register_test_device();
    store_account(&mut f.provider);
    let result = f
        .provider
        .mutate_playlist_items(
            REF,
            PlaylistItemMutationAction::Add,
            &request(&[901, 800, 900], "A"),
        )
        .await
        .unwrap();
    assert_eq!(
        result.item_refs.iter().map(|r| r.id()).collect::<Vec<_>>(),
        ["901", "800", "900"]
    );
    assert_eq!(result.extensions["affected_occurrences"], 2);
    let requests = f.requests.await.unwrap();
    let write = requests
        .iter()
        .find(|r| r.starts_with("POST /cloudlist.service/v6/add_song?"))
        .unwrap();
    let b = body(write);
    assert_eq!(b["data"].as_array().unwrap().len(), 2);
    assert_eq!(b["data"][0]["mixsongid"], 901);
    assert_eq!(b["data"][1]["mixsongid"], 900);
}

#[tokio::test]
async fn write_readback_rejects_target_misses_unrelated_changes_and_acknowledgement_mismatch() {
    for case in [
        "not_removed",
        "unrelated_removed",
        "unrelated_added",
        "reordered",
        "wrong_count",
        "wrong_version",
        "renamed",
    ] {
        let before = vec![row(1, 800, 0), row(81, 900, 1), row(2, 801, 2)];
        let after = match case {
            "not_removed" => before.clone(),
            "unrelated_removed" => vec![],
            "unrelated_added" => vec![row(1, 800, 0), row(2, 801, 1), row(3, 999, 2)],
            "reordered" => vec![row(2, 801, 0), row(1, 800, 1)],
            _ => vec![row(1, 800, 0), row(2, 801, 1)],
        };
        let mut frames = start();
        frames.extend(snapshot(before, 1));
        frames.push(acknowledgement(
            1,
            if case == "wrong_version" { 3 } else { 2 },
            if case == "wrong_count" { 99 } else { 2 },
        ));
        let after_frames = snapshot(after, 2);
        if case == "renamed" {
            // Replace both v8 metadata pages consistently, so only the write delta check rejects it.
            let mut p = list_row(37, 0);
            p["is_def"] = json!(2);
            p["list_ver"] = json!(2);
            p["name"] = json!("Concurrent rename");
            frames.push(library_page(vec![p.clone()]).into());
            frames.extend(after_frames.into_iter().skip(1).take(1));
            frames.push(library_page(vec![p]).into());
        } else {
            frames.extend(after_frames);
        }
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let e = f
            .provider
            .set_track_subscription("900", false, Some("A"))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!e.retryable);
        let requests = f.requests.await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /v4/delete_songs?"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn dispatch_and_later_auth_errors_keep_correct_credential_ownership_and_uncertain_write_state()
 {
    for caller in [false, true] {
        for code in [20010, 20017] {
            let mut frames = start();
            frames.extend(snapshot(vec![row(81, 900, 0)], 1));
            frames.push(
                raw(json!({"status":0,"error_code":code,"data":"write-error-payload"})).into(),
            );
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut e = provider
                .set_track_subscription("900", false, if caller { None } else { Some("A") })
                .await
                .unwrap_err();
            assert_eq!(
                e.code,
                if code == 20017 {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert_eq!(e.details["write_requests_dispatched"], 1);
            assert!(!e.retryable);
            assert_eq!(
                e.take_caller_credential_update().is_some(),
                caller && code != 20017
            );
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            assert_eq!(f.requests.await.unwrap().len(), 6);
        }
    }
}

#[tokio::test]
async fn changed_playlist_during_catalogue_lookup_stops_before_dispatch() {
    let before = vec![row(1, 800, 0)];
    let mut frames = start();
    frames.extend(snapshot(before, 1));
    frames.extend(catalogue(900));
    frames.extend(snapshot(vec![row(1, 800, 0), row(2, 999, 1)], 2));
    let mut f = server(frames).await;
    f.provider.client.register_test_device();
    store_account(&mut f.provider);
    let e = f
        .provider
        .set_track_subscription("900", true, Some("A"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert!(e.details.get("write_outcome").is_none());
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 10);
    assert!(
        !requests
            .iter()
            .any(|r| r.starts_with("POST /cloudlist.service/v6/add_song?"))
    );
}

#[tokio::test]
async fn relogin_during_write_or_final_readback_discards_late_result_without_repeating_write() {
    for final_readback in [false, true] {
        for caller in [false, true] {
            let mut frames = start();
            frames.extend(snapshot(vec![row(81, 900, 0)], 1));
            let (last, resume) = if final_readback {
                frames.push(acknowledgement(1, 2, 0));
                let mut after = snapshot(vec![], 2);
                after.pop();
                frames.extend(after);
                let mut p = list_row(37, 0);
                p["is_def"] = json!(2);
                p["list_ver"] = json!(2);
                paused(library_page(vec![p]))
            } else {
                paused(reply(
                    json!({"userid":111,"listid":37,"list_ver":2,"pre_list_ver":1,"count":0}),
                ))
            };
            frames.push(last);
            let n = frames.len();
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let p = provider.clone();
            let task = tokio::spawn(async move {
                p.set_track_subscription("900", false, if caller { None } else { Some("A") })
                    .await
            });
            for _ in 0..n {
                f.seen.recv().await.unwrap();
            }
            let replacement = credential("111", "new-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                    Some(replacement.clone());
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut e = task.await.unwrap().unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict);
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(e.take_caller_credential_update().is_none());
            if caller {
                assert_eq!(read(&store, "A"), saved);
            } else {
                assert_eq!(read(&store, "A"), replacement);
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.starts_with("POST /v4/delete_songs?"))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn invalid_mutation_scope_kind_and_references_never_reach_the_network() {
    let mut f = server(vec![]).await;
    store_account(&mut f.provider);
    for ids in [vec![], vec![900, 900], vec![0], (1..=101).collect()] {
        assert_eq!(
            f.provider
                .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &request(&ids, "A"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .mutate_playlist_items(
                "cloudlist:111:1:37",
                PlaylistItemMutationAction::Remove,
                &request(&[900], "A")
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.provider
            .mutate_playlist_items(
                "cloudlist:222:0:37",
                PlaylistItemMutationAction::Remove,
                &request(&[900], "A")
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let mut r = request(&[900], "A");
    r.kind = PlaylistItemKind::Video;
    assert_eq!(
        f.provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &r)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    r.kind = PlaylistItemKind::Track;
    r.item_refs = vec![ResourceRef::new(Platform::Migu, "900").unwrap()];
    assert_eq!(
        f.provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &r)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn partial_deletion_and_conflicting_ack_versions_stop_without_followup_or_retry() {
    for caller in [false, true] {
        for case in ["first_previous", "second_previous", "second_failure"] {
            let mut frames = start();
            frames.extend(snapshot((1..=302).map(|i| row(i, 900, i)).collect(), 1));
            if case == "first_previous" {
                frames.push(acknowledgement(999, 2, 2));
            } else {
                frames.push(acknowledgement(1, 2, 2));
                frames.push(if case == "second_previous" {
                    acknowledgement(999, 3, 0)
                } else {
                    raw(json!({"status":0,"error_code":20010})).into()
                });
            }
            let expected = if case == "first_previous" { 1 } else { 2 };
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut e = provider
                .set_track_subscription("900", false, if caller { None } else { Some("A") })
                .await
                .unwrap_err();
            assert_eq!(
                e.code,
                if case == "second_failure" {
                    ErrorCode::UpstreamError
                } else {
                    ErrorCode::Conflict
                }
            );
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert_eq!(e.details["write_requests_dispatched"], expected);
            assert!(!e.retryable);
            assert_eq!(
                e.take_caller_credential_update().is_some(),
                caller && case == "second_failure"
            );
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            let requests = f.requests.await.unwrap();
            let writes = requests
                .iter()
                .filter(|r| r.starts_with("POST /v4/delete_songs?"))
                .collect::<Vec<_>>();
            assert_eq!(writes.len(), expected);
            assert_eq!(body(writes[0])["data"].as_array().unwrap().len(), 300);
            if expected == 2 {
                assert_eq!(
                    body(writes[1])["data"],
                    json!([{"fileid":301},{"fileid":302}])
                );
            }
        }
    }
}

#[tokio::test]
async fn anonymous_catalogue_failures_cannot_invalidate_native_account_or_dispatch_writes() {
    for caller in [false, true] {
        for mismatch in [false, true] {
            let mut frames = start();
            frames.extend(snapshot(vec![], 1));
            frames.push(if mismatch {
                catalogue(901).remove(0)
            } else {
                raw(json!({"status":0,"error_code":20017}))
                    .replacen("200 OK", "401 Unauthorized", 1)
                    .into()
            });
            let mut f = server(frames).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut e = provider
                .set_track_subscription("900", true, if caller { None } else { Some("A") })
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::UpstreamError);
            assert!(e.details.get("write_outcome").is_none());
            assert_eq!(e.take_caller_credential_update().is_some(), caller);
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), 6);
            assert!(
                !requests
                    .iter()
                    .any(|r| r.starts_with("POST /cloudlist.service/v6/add_song?"))
            );
        }
    }
}

#[tokio::test]
async fn concept_track_add_rejects_legacy_favorite_contract_without_io() {
    let mut f = server(vec![]).await;
    let store = store_account(&mut f.provider);
    let mut session = read(&store, "A").native().session.clone();
    session.client = crate::KugouLoginClient::Concept;
    store
        .put(
            &KugouCredential::verified(session)
                .unwrap()
                .stored("A")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        f.provider
            .set_track_subscription("900", true, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
}
