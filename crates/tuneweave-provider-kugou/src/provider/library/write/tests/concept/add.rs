use super::*;
use std::collections::BTreeMap;

fn hint() -> Frame {
    reply(json!([{"__status":1,"base":{"album_audio_id":900,"album_id":42}}])).into()
}
fn song(id: u64) -> Value {
    json!({"base":{"album_audio_id":id,"album_id":42,"is_publish":1,
        "author_name":"Artist","audio_name":"Song"},"audio_info":{
        "hash":"abcdef0123456789abcdef0123456789","extname":"mp3",
        "duration":216789,"filesize":120000,"bitrate":128}})
}
fn album(rows: Vec<Value>, total: u64) -> String {
    reply(json!({"total":total,"songs":rows}))
}
fn receipt() -> String {
    reply(
        json!({"userid":111,"listid":37,"list_ver":8,"pre_list_ver":7,"count":2,
        "info":[{"fileid":99,"sort":0,"name":"Artist - Song.mp3",
        "hash":"abcdef0123456789abcdef0123456789","album_id":"42","mixsongid":900}]}),
    )
}
fn after(file: u64) -> Vec<Value> {
    let mut track = row(file, 900, 0);
    track["album_id"] = json!(42);
    vec![track, row(70, 800, 1)]
}
fn add_request(caller: bool) -> PlaylistItemMutationRequest {
    remove_request(&[900], caller)
}

#[tokio::test]
async fn concept_add_provider_uses_complete_anonymous_album_and_exact_readback_for_both_owners() {
    for caller in [false, true] {
        let before = vec![row(70, 800, 0)];
        let mut frames = start();
        frames.extend(ordinary_snapshot(before.clone(), 7, list(7)));
        frames.push(hint());
        frames.push(album((1000..1020).map(song).collect(), 21).into());
        frames.push(album(vec![song(900)], 21).into());
        frames.extend(ordinary_snapshot(before, 7, list(7)));
        frames.push(receipt().into());
        frames.extend(ordinary_snapshot(after(99), 8, list(8)));
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
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(caller))
            .await
            .unwrap();
        assert_eq!(result.cloud_track_count, Some(2));
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(p.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), count);
        let public = requests
            .iter()
            .filter(|r| r.starts_with("POST /kmr/v1/album_songlist?"))
            .collect::<Vec<_>>();
        assert_eq!(public.len(), 2);
        for (index, r) in public.iter().enumerate() {
            let (head, wire) = r.split_once("\r\n\r\n").unwrap();
            assert_eq!(
                body(r),
                json!({"fields":"musical","album_id":"42","page":(index+1).to_string(),"pagesize":"20","is_buy":"0"})
            );
            assert!(head.to_ascii_lowercase().contains("kg-tid: 221"));
            assert!(!r.contains("next"));
            assert!(!r.contains("token"));
            assert!(!r.contains("userid"));
            assert!(!head.to_ascii_lowercase().contains("cookie:"));
            let target = head
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
            let mut query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
            assert_eq!(query["appid"], "3116");
            let signature = query.remove("signature").unwrap();
            let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            assert_eq!(
                signature,
                crate::signing::concept_signature(&query, wire.as_bytes())
            );
        }
        assert!(
            !requests
                .iter()
                .any(|r| r.starts_with("POST /v1/audio/audio?"))
        );
        let writes = requests
            .iter()
            .filter(|r| r.starts_with("POST /cloudlist.service/v4/add_song?"))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 1);
        let wire = body(writes[0]);
        assert_eq!(wire["userid"], "111");
        assert_eq!(wire["list_ver"], 7);
        assert_eq!(wire["data"][0]["timelen"], 216789);
        assert_eq!(wire["data"][0]["bitrate"], 128);
        assert_eq!(wire["data"][0]["album_id"], "42");
        assert!(!wire.as_object().unwrap().contains_key("scene"));
    }
}

#[tokio::test]
async fn concept_add_provider_rejects_ambiguous_missing_or_changed_catalogue_before_write() {
    for case in [
        "duplicate",
        "missing",
        "units",
        "changed_total",
        "short_page",
    ] {
        let mut frames = start();
        frames.extend(ordinary_snapshot(vec![row(70, 800, 0)], 7, list(7)));
        frames.push(hint());
        match case {
            "duplicate" => frames.push(album(vec![song(900), song(900)], 2).into()),
            "missing" => frames.push(album(vec![song(901)], 1).into()),
            "units" => {
                let mut value = song(900);
                value["audio_info"]
                    .as_object_mut()
                    .unwrap()
                    .remove("duration");
                frames.push(album(vec![value], 1).into());
            }
            "changed_total" => {
                frames.push(album((1000..1020).map(song).collect(), 21).into());
                frames.push(album(vec![song(900), song(901)], 22).into());
            }
            _ => frames.push(album(vec![song(900)], 21).into()),
        }
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let error = f
            .provider
            .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(false))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError, "{case}");
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
async fn concept_add_provider_rejects_favorites_systems_and_duplicate_existing_rows() {
    let mut f = server(vec![]).await;
    concept_store(&mut f.provider);
    assert_eq!(
        f.provider
            .set_track_subscription("900", true, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
    for duplicate in [false, true] {
        let mut selected = list(7);
        if !duplicate {
            selected["is_def"] = json!(2);
        }
        let mut frames = start();
        frames.extend(ordinary_snapshot(
            vec![row(81, 900, 0), row(82, 900, 1)],
            7,
            selected,
        ));
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        assert_eq!(
            f.provider
                .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(false))
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn concept_add_provider_rejects_readback_with_different_raw_receipt_identity() {
    let before = vec![row(70, 800, 0)];
    let mut frames = start();
    frames.extend(ordinary_snapshot(before.clone(), 7, list(7)));
    frames.push(hint());
    frames.push(album(vec![song(900)], 1).into());
    frames.extend(ordinary_snapshot(before, 7, list(7)));
    frames.push(receipt().into());
    frames.extend(ordinary_snapshot(after(100), 8, list(8)));
    let mut f = server(frames).await;
    concept_store(&mut f.provider);
    let error = f
        .provider
        .mutate_playlist_items(REF, PlaylistItemMutationAction::Add, &add_request(false))
        .await
        .unwrap_err();
    assert_eq!(error.details["write_outcome"], "unconfirmed");
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

#[tokio::test]
async fn concept_add_provider_relogin_rejects_late_public_or_write_responses_for_both_owners() {
    for caller in [false, true] {
        for at_write in [false, true] {
            for failed in [false, true] {
                let before = vec![row(70, 800, 0)];
                let mut frames = start();
                frames.extend(ordinary_snapshot(before.clone(), 7, list(7)));
                frames.push(hint());
                if at_write {
                    frames.push(album(vec![song(900)], 1).into());
                    frames.extend(ordinary_snapshot(before, 7, list(7)));
                }
                let (frame, resume) = paused(if failed {
                    raw(json!({"status":0,"error_code":20017}))
                } else if at_write {
                    receipt()
                } else {
                    album(vec![song(900)], 1)
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
                        &add_request(caller),
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
                assert_eq!(error.details.get("write_outcome").is_some(), at_write);
                assert!(error.take_caller_credential_update().is_none());
                assert!(provider.take_response_credential().unwrap().is_none());
                assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
                assert_eq!(f.requests.await.unwrap().len(), count);
            }
        }
    }
}

mod batch;
