use super::super::tests::{row as list_row, store_account};
use super::*;
use crate::account::cloud::tests::{self as wire, Frame, encrypted, plaintext};
use crate::provider::session::tests::{credential, exchange, profile, read, reply};
use tuneweave_core::ResourceRef;

const REF: &str = "cloudlist:111:1:7";
fn track(file: u64, song: Option<u64>, sort: u64) -> Value {
    json!({"fileid":file,"mixsongid":song,"name":format!("Song {file}"),"sort":sort})
}
fn library(rows: Vec<Value>, version: u64) -> Frame {
    reply(json!({"userid":111,"total_ver":version,"list_count":2,"collect_count":1,"info":rows}))
        .into()
}
fn list(kind: u8, ver: u64) -> Value {
    let mut p = list_row(7, kind);
    p["list_ver"] = json!(ver);
    p["sort"] = json!(0);
    p
}
fn snapshot(rows: Vec<Value>, kind: u8, ver: u64) -> Vec<Frame> {
    let mut result = vec![library(vec![list(kind, ver)], 9 + ver)];
    let total = rows.len();
    if rows.is_empty() {
        result.push(
            reply(json!({"userid":111,"listid":7,"type":kind,"list_ver":ver,"count":0,"info":[]}))
                .into(),
        );
    }
    for chunk in rows.chunks(300) {
        result.push(reply(json!({"userid":111,"listid":7,"type":kind,"list_ver":ver,"count":total,"info":chunk})).into());
    }
    result.push(library(vec![list(kind, ver)], 9 + ver));
    result
}
fn start() -> Vec<Frame> {
    vec![exchange("111", "next").into(), profile("111").into()]
}
fn order(ids: &[u64], account: Option<&str>) -> PlaylistTrackOrderRequest {
    PlaylistTrackOrderRequest {
        track_refs: ids
            .iter()
            .map(|id| ResourceRef::new(Platform::Kugou, id.to_string()).unwrap())
            .collect(),
        account: account.map(str::to_owned),
    }
}
fn ack(kind: u8) -> Frame {
    encrypted(json!({"userid":111,"listid":7,"type":kind,"list_ver":4,"pre_list_ver":3}))
}

#[tokio::test]
async fn raw_occurrences_keep_unresolved_entries_and_duplicate_songs_across_pages() {
    let mut rows = (1..=305)
        .map(|n| track(n, Some(900 + n % 5), 305 - n))
        .collect::<Vec<_>>();
    rows[0]["mixsongid"] = Value::Null;
    let mut frames = start();
    frames.extend(snapshot(rows, 1, 3));
    let f = wire::server(frames).await;
    let mut provider = KugouProvider::from_client(f.client);
    store_account(&mut provider);
    let page = provider
        .playlist_track_occurrences(
            REF,
            &PageRequest {
                limit: 10,
                offset: 295,
                account: Some("A".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 10);
    assert_eq!(page.pagination.total, Some(305));
    assert_eq!(page.pagination.extensions["upstream_pages_fetched"], 2);
    assert_eq!(page.items[9].id, "entry:111:1:7:1");
    assert_eq!(page.items[9].position, 304);
    assert!(page.items[9].track.is_none());
    assert_eq!(
        page.items[0].track.as_ref().unwrap().id,
        page.items[5].track.as_ref().unwrap().id
    );
    assert!(
        page.pagination.extensions["source_snapshot_id"]
            .as_str()
            .unwrap()
            .starts_with("kugou-cloudlist-raw-")
    );
    assert_eq!(f.requests.await.unwrap().len(), 6);
}

#[tokio::test]
async fn occurrence_sort_roundtrip_preserves_unknown_entries_for_both_native_clients_and_library_types()
 {
    for client in [
        crate::KugouLoginClient::Standard,
        crate::KugouLoginClient::Concept,
    ] {
        for kind in [0, 1] {
            let id = format!("cloudlist:111:{kind}:7");
            let before = vec![
                track(81, Some(901), 0),
                track(82, None, 1),
                track(83, Some(901), 2),
            ];
            let mut after = vec![before[2].clone(), before[1].clone(), before[0].clone()];
            for (i, row) in after.iter_mut().enumerate() {
                row["sort"] = json!(i);
            }
            let mut frames = start();
            frames.extend(snapshot(before.clone(), kind, 3));
            frames.extend(start());
            frames.extend(snapshot(before, kind, 3));
            frames.push(ack(kind));
            frames.extend(snapshot(after, kind, 4));
            let f = wire::server(frames).await;
            let mut provider = KugouProvider::from_client(f.client);
            let store = store_account(&mut provider);
            let mut source = read(&store, "A").native().session.clone();
            source.client = client;
            store
                .put(
                    &KugouCredential::verified(source)
                        .unwrap()
                        .stored("A")
                        .unwrap(),
                )
                .unwrap();
            let page = provider
                .playlist_track_occurrences(
                    &id,
                    &PageRequest {
                        limit: 100,
                        offset: 0,
                        account: Some("A".into()),
                    },
                )
                .await
                .unwrap();
            let result = provider
                .reorder_playlist_occurrences(
                    &id,
                    &PlaylistOccurrenceOrderRequest {
                        occurrence_ids: page.items.into_iter().rev().map(|e| e.id).collect(),
                        snapshot_id: page.pagination.extensions["source_snapshot_id"]
                            .as_str()
                            .unwrap()
                            .into(),
                        account: Some("A".into()),
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.occurrence_ids[0], format!("entry:111:{kind}:7:83"));
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            let all = f.requests.await.unwrap();
            assert_eq!(all.len(), 14);
            assert_eq!(
                plaintext(&all[10])["data"],
                json!([{"fileid":83,"sort":0},{"fileid":82,"sort":1},{"fileid":81,"sort":2}])
            );
        }
    }
}

#[tokio::test]
async fn catalogue_sort_maps_duplicate_songs_fifo_and_retains_caller_or_server_rotations() {
    for caller in [false, true] {
        let before = vec![
            track(81, Some(901), 0),
            track(82, Some(902), 1),
            track(83, Some(901), 2),
        ];
        let after = vec![
            track(81, Some(901), 0),
            track(83, Some(901), 1),
            track(82, Some(902), 2),
        ];
        let mut frames = start();
        frames.extend(snapshot(before, 1, 3));
        frames.push(ack(1));
        frames.extend(snapshot(after, 1, 4));
        let f = wire::server(frames).await;
        let mut base = KugouProvider::from_client(f.client);
        let store = store_account(&mut base);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            base.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            base
        };
        let r = provider
            .reorder_playlist_tracks(
                REF,
                &order(&[901, 901, 902], if caller { None } else { Some("A") }),
            )
            .await
            .unwrap();
        assert_eq!(r.track_refs[0], r.track_refs[1]);
        assert_eq!(r.extensions["confirmed"], true);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(provider.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 9);
        assert_eq!(plaintext(&all[5])["data"][1]["fileid"], 83);
    }
}

#[tokio::test]
async fn incomplete_foreign_duplicate_stale_and_unresolved_orders_fail_before_any_cloud_write() {
    for case in [
        "missing",
        "duplicate",
        "foreign_song",
        "unresolved",
        "stale",
        "unknown_occurrence",
    ] {
        let mut before = vec![track(81, Some(901), 0), track(82, Some(902), 1)];
        if case == "unresolved" {
            before[1]["mixsongid"] = Value::Null;
        }
        let mut frames = start();
        frames.extend(snapshot(before, 1, 3));
        let f = wire::server(frames).await;
        let mut p = KugouProvider::from_client(f.client);
        store_account(&mut p);
        let error = if matches!(case, "stale" | "unknown_occurrence") {
            p.reorder_playlist_occurrences(
                REF,
                &PlaylistOccurrenceOrderRequest {
                    occurrence_ids: vec![
                        "entry:111:1:7:81".into(),
                        format!("entry:111:1:7:{}", if case == "stale" { 82 } else { 99 }),
                    ],
                    snapshot_id: format!("kugou-cloudlist-raw-{}", "0".repeat(32)),
                    account: Some("A".into()),
                },
            )
            .await
            .unwrap_err()
        } else {
            p.reorder_playlist_tracks(
                REF,
                &order(
                    match case {
                        "missing" => &[901],
                        "duplicate" => &[901, 901],
                        "foreign_song" => &[901, 999],
                        _ => &[902, 901],
                    },
                    Some("A"),
                ),
            )
            .await
            .unwrap_err()
        };
        assert!(matches!(
            error.code,
            ErrorCode::InvalidRequest | ErrorCode::CapabilityNotSupported | ErrorCode::Conflict
        ));
        assert!(error.details.get("write_outcome").is_none(), "{case}");
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
    let f = wire::server(vec![]).await;
    let p = KugouProvider::from_client(f.client);
    for ids in [
        vec!["entry:222:1:7:81".into()],
        vec!["entry:111:1:7:81".into(), "entry:111:1:7:81".into()],
    ] {
        assert_eq!(
            p.reorder_playlist_occurrences(
                REF,
                &PlaylistOccurrenceOrderRequest {
                    occurrence_ids: ids,
                    snapshot_id: format!("kugou-cloudlist-raw-{}", "0".repeat(32)),
                    account: Some("A".into())
                }
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn observed_catalogue_noops_including_empty_playlists_make_no_write() {
    for empty in [false, true] {
        let rows = if empty {
            vec![]
        } else {
            vec![track(81, Some(901), 0)]
        };
        let mut frames = start();
        frames.extend(snapshot(rows, 1, 3));
        let f = wire::server(frames).await;
        let mut p = KugouProvider::from_client(f.client);
        store_account(&mut p);
        let result = p
            .reorder_playlist_tracks(REF, &order(if empty { &[] } else { &[901] }, Some("A")))
            .await
            .unwrap();
        assert_eq!(result.extensions["write_requests_dispatched"], 0);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn sort_readback_rejects_wrong_order_metadata_versions_and_identity_without_retry() {
    for case in [
        "order",
        "metadata",
        "missing",
        "wrong_ack_version",
        "wrong_previous",
        "plain_success",
        "auth",
    ] {
        let before = vec![track(81, Some(901), 0), track(82, Some(902), 1)];
        let mut after = vec![track(82, Some(902), 0), track(81, Some(901), 1)];
        if case == "order" {
            after = before.clone();
        }
        if case == "metadata" {
            after[1]["name"] = json!("Changed elsewhere");
        }
        if case == "missing" {
            after.pop();
        }
        let mut frames = start();
        frames.extend(snapshot(before, 1, 3));
        frames.push(match case {
            "plain_success"=>reply(json!({})).into(),
            "auth"=>crate::provider::session::tests::raw(json!({"status":0,"error_code":20017})).into(),
            _=>encrypted(json!({"list_ver":if case=="wrong_ack_version"{8}else{4},"pre_list_ver":if case=="wrong_previous"{8}else{3}})),
        });
        if !matches!(case, "plain_success" | "auth") {
            frames.extend(snapshot(after, 1, 4));
        }
        let count = frames.len();
        let f = wire::server(frames).await;
        let mut p = KugouProvider::from_client(f.client);
        let store = store_account(&mut p);
        let p = p
            .caller_scope(&read(&store, "A").caller().unwrap())
            .unwrap();
        let e = p
            .reorder_playlist_tracks(REF, &order(&[902, 901], None))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!e.retryable);
        assert_eq!(
            p.take_response_credential().unwrap().is_some(),
            case == "plain_success"
        );
        assert_eq!(f.requests.await.unwrap().len(), count);
    }
}

fn directory_row(id: u64, kind: u8, sort: u64) -> Value {
    let mut p = list_row(id, kind);
    p["sort"] = json!(sort);
    p
}
fn directory_order(ids: &[(u8, u64)]) -> PlaylistOrderRequest {
    PlaylistOrderRequest {
        playlist_refs: ids
            .iter()
            .map(|(kind, id)| {
                ResourceRef::new(Platform::Kugou, format!("cloudlist:111:{kind}:{id}")).unwrap()
            })
            .collect(),
        account: Some("A".into()),
    }
}
#[tokio::test]
async fn directory_order_uses_complete_categories_and_independent_zero_based_positions() {
    for both in [false, true] {
        let before = vec![
            directory_row(1, 0, 0),
            directory_row(7, 0, 1),
            directory_row(8, 1, 0),
            directory_row(9, 1, 1),
        ];
        let after = vec![
            directory_row(1, 0, 1),
            directory_row(7, 0, 0),
            directory_row(8, 1, u64::from(both)),
            directory_row(9, 1, u64::from(!both)),
        ];
        let mut frames = start();
        frames.push(library(before, 9));
        frames.push(encrypted(json!({"pre_total_ver":9,"total_ver":10})));
        frames.push(library(after, 10));
        let f = wire::server(frames).await;
        let mut p = KugouProvider::from_client(f.client);
        store_account(&mut p);
        let request = directory_order(if both {
            &[(0, 7), (1, 9), (0, 1), (1, 8)]
        } else {
            &[(0, 7), (0, 1)]
        });
        let result = p.reorder_account_playlists(&request).await.unwrap();
        assert_eq!(result.extensions["confirmed"], true);
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 5);
        let body = plaintext(&all[3]);
        assert_eq!(body["data"][0]["sort"], 0);
        if both {
            assert_eq!(body["data"][2]["sort"], 0);
            assert_eq!(body["data"][2]["type"], 1);
        }
    }
}
#[tokio::test]
async fn directory_rejects_partial_categories_and_detects_unselected_or_metadata_changes() {
    for case in [
        "partial",
        "noop",
        "unselected_sort",
        "metadata",
        "extra",
        "wrong_order",
    ] {
        let before = vec![
            directory_row(1, 0, 0),
            directory_row(7, 0, 1),
            directory_row(8, 1, 0),
        ];
        let mut after = vec![
            directory_row(1, 0, 1),
            directory_row(7, 0, 0),
            directory_row(8, 1, 0),
        ];
        if case == "unselected_sort" {
            after[2]["sort"] = json!(99);
        }
        if case == "metadata" {
            after[0]["name"] = json!("Changed elsewhere");
        }
        if case == "extra" {
            after.push(directory_row(99, 1, 1));
        }
        if case == "wrong_order" {
            after = before.clone();
        }
        let mut frames = start();
        frames.push(library(before, 9));
        if !matches!(case, "partial" | "noop") {
            frames.push(encrypted(json!({})));
            frames.push(library(after, 10));
        }
        let count = frames.len();
        let f = wire::server(frames).await;
        let mut p = KugouProvider::from_client(f.client);
        store_account(&mut p);
        let r = p
            .reorder_account_playlists(&directory_order(match case {
                "partial" => &[(0, 7)],
                "noop" => &[(0, 1), (0, 7)],
                _ => &[(0, 7), (0, 1)],
            }))
            .await;
        if case == "noop" {
            assert_eq!(r.unwrap().extensions["write_requests_dispatched"], 0);
        } else {
            let e = r.unwrap_err();
            assert_eq!(e.details.get("write_outcome").is_some(), case != "partial");
        }
        assert_eq!(f.requests.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn late_relogin_after_cloud_dispatch_cannot_overwrite_or_return_old_credentials() {
    let mut frames = start();
    frames.extend(snapshot(
        vec![track(81, Some(901), 0), track(82, Some(902), 1)],
        1,
        3,
    ));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut response = ack(1);
    response.gate = Some(rx);
    frames.push(response);
    let mut f = wire::server(frames).await;
    let mut p = KugouProvider::from_client(f.client);
    let store = store_account(&mut p);
    let task = tokio::spawn(async move {
        p.reorder_playlist_tracks(REF, &order(&[902, 901], Some("A")))
            .await
    });
    for _ in 0..6 {
        f.seen.recv().await.unwrap();
    }
    let fresh = credential("111", "relogin");
    store.put(&fresh.stored("A").unwrap()).unwrap();
    tx.send(()).unwrap();
    let e = task.await.unwrap().unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert_eq!(e.details["write_outcome"], "unconfirmed");
    assert_eq!(read(&store, "A"), fresh);
    assert_eq!(f.requests.await.unwrap().len(), 6);
}

#[tokio::test]
async fn raw_order_rejects_unknown_file_ids_with_current_snapshot_and_preserves_unresolved_metadata()
 {
    for case in ["unknown_file", "changed_unknown_metadata", "move_unknown"] {
        let before = vec![track(81, Some(901), 0), track(82, None, 1)];
        let mut after = vec![track(82, None, 0), track(81, Some(901), 1)];
        if case == "changed_unknown_metadata" {
            after[0]["name"] = json!("Concurrent metadata edit");
        }
        let mut frames = start();
        frames.extend(snapshot(before.clone(), 1, 3));
        frames.extend(start());
        frames.extend(snapshot(before, 1, 3));
        if case != "unknown_file" {
            frames.push(ack(1));
            frames.extend(snapshot(after, 1, 4));
        }
        let count = frames.len();
        let f = wire::server(frames).await;
        let mut p = KugouProvider::from_client(f.client);
        store_account(&mut p);
        let page = p
            .playlist_track_occurrences(
                REF,
                &PageRequest {
                    limit: 100,
                    offset: 0,
                    account: Some("A".into()),
                },
            )
            .await
            .unwrap();
        let wanted = vec![
            format!(
                "entry:111:1:7:{}",
                if case == "unknown_file" { 99 } else { 82 }
            ),
            "entry:111:1:7:81".into(),
        ];
        let result = p
            .reorder_playlist_occurrences(
                REF,
                &PlaylistOccurrenceOrderRequest {
                    occurrence_ids: wanted.clone(),
                    snapshot_id: page.pagination.extensions["source_snapshot_id"]
                        .as_str()
                        .unwrap()
                        .into(),
                    account: Some("A".into()),
                },
            )
            .await;
        if case == "move_unknown" {
            assert_eq!(result.unwrap().occurrence_ids, wanted);
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                error.code,
                if case == "unknown_file" {
                    ErrorCode::InvalidRequest
                } else {
                    ErrorCode::Conflict
                }
            );
            assert_eq!(
                error.details.get("write_outcome").is_some(),
                case != "unknown_file"
            );
        }
        assert_eq!(f.requests.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn ordering_submits_every_occurrence_across_physical_page_boundaries() {
    let before = (1..=305)
        .map(|id| track(id, Some(900 + id), id - 1))
        .collect::<Vec<_>>();
    let mut after = before.iter().cloned().rev().collect::<Vec<_>>();
    for (i, row) in after.iter_mut().enumerate() {
        row["sort"] = json!(i);
    }
    let mut frames = start();
    frames.extend(snapshot(before, 1, 3));
    frames.push(ack(1));
    frames.extend(snapshot(after, 1, 4));
    let f = wire::server(frames).await;
    let mut p = KugouProvider::from_client(f.client);
    store_account(&mut p);
    let r = p
        .reorder_playlist_tracks(
            REF,
            &order(&(901..=1205).rev().collect::<Vec<_>>(), Some("A")),
        )
        .await
        .unwrap();
    assert_eq!(r.track_refs.len(), 305);
    let all = f.requests.await.unwrap();
    assert_eq!(all.len(), 11);
    let body = plaintext(&all[6]);
    assert_eq!(body["data"].as_array().unwrap().len(), 305);
    assert_eq!(body["data"][299]["fileid"], 6);
    assert_eq!(body["data"][304]["fileid"], 1);
}
