use super::*;
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{
    Frame, credential, exchange, paused, profile, raw, read, reply, server,
};

pub(super) fn row(kind: Kind, id: u64) -> serde_json::Value {
    match kind {
        Kind::Tracks => {
            json!({"id":id,"album_audio_id":901+id%3,"good_scid":100+id,"songname":format!("Song {id}"),"author_name":"Singer"})
        }
        Kind::Albums => {
            json!({"id":id,"album_id":70+id%3,"album_name":format!("Album {id}"),"singer_name":"Singer"})
        }
    }
}
fn page(kind: Kind, rows: Vec<serde_json::Value>, total: usize) -> String {
    reply(json!({"userid":111,"goods":rows,"total":total,"pagesize":kind.page_size()}))
}
pub(super) fn scan(kind: Kind, rows: &[serde_json::Value]) -> Vec<Frame> {
    if rows.is_empty() {
        return vec![page(kind, vec![], 0).into()];
    }
    rows.chunks(kind.page_size())
        .map(|chunk| page(kind, chunk.to_vec(), rows.len()).into())
        .collect()
}
pub(super) fn start() -> Vec<Frame> {
    vec![exchange("111", "next").into(), profile("111").into()]
}
pub(super) fn request(account: Option<&str>, offset: u32) -> PageRequest {
    PageRequest {
        limit: 10,
        offset,
        account: account.map(str::to_owned),
    }
}

#[tokio::test]
async fn purchases_read_all_pages_twice_preserve_repeated_catalogue_entries_and_slice_after_confirmation()
 {
    for kind in [Kind::Tracks, Kind::Albums] {
        let mut rows = (1..=kind.page_size() + 4)
            .map(|id| row(kind, id as u64))
            .collect::<Vec<_>>();
        rows[0]
            .as_object_mut()
            .unwrap()
            .remove(if kind == Kind::Tracks {
                "album_audio_id"
            } else {
                "album_id"
            });
        let mut frames = start();
        frames.extend(scan(kind, &rows));
        frames.extend(scan(kind, &rows));
        let mut f = server(frames).await;
        let store = store_account(&mut f.provider);
        let other = read(&store, "B");
        let result = f
            .provider
            .native_purchases(kind, &request(Some("A"), kind.page_size() as u32 - 1))
            .await
            .unwrap();
        assert_eq!(result.items.len(), 5);
        assert_eq!(result.pagination.total, Some(rows.len() as u64));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.extensions["unresolved_entries"], 1);
        assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 4);
        assert_eq!(
            result.items[0].catalogue_id(),
            result.items[3].catalogue_id()
        );
        assert_ne!(
            result.items[0].identity().unwrap(),
            result.items[3].identity().unwrap()
        );
        assert_eq!(read(&store, "A").native().session.token, "next");
        assert_eq!(read(&store, "B"), other);
        assert!(f.provider.take_response_credential().unwrap().is_none());
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 6);
        for (index, r) in all[2..].iter().enumerate() {
            let body: serde_json::Value =
                serde_json::from_str(r.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["page"], index % 2 + 1);
            assert_eq!(body["token"], "next");
        }
    }
}

#[tokio::test]
async fn purchase_wrappers_deliver_complete_caller_results_for_standard_and_concept_without_persisting()
 {
    for client in [
        crate::KugouLoginClient::Standard,
        crate::KugouLoginClient::Concept,
    ] {
        for kind in [Kind::Tracks, Kind::Albums] {
            let rows = vec![row(kind, 1)];
            let mut frames = start();
            frames.extend(scan(kind, &rows));
            frames.extend(scan(kind, &rows));
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
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
            let saved = read(&store, "A");
            let p = f.provider.caller_scope(&saved.caller().unwrap()).unwrap();
            if kind == Kind::Tracks {
                let result = p.account_purchased_tracks(&request(None, 0)).await.unwrap();
                assert_eq!(result.items[0].track.as_ref().unwrap().id, "902");
            } else {
                let result = p.account_purchased_albums(&request(None, 0)).await.unwrap();
                assert_eq!(result.items[0].album.as_ref().unwrap().id, "71");
            }
            assert_eq!(read(&store, "A"), saved);
            assert!(p.take_response_credential().unwrap().is_some());
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn empty_and_out_of_range_purchase_windows_still_verify_the_complete_library() {
    for kind in [Kind::Tracks, Kind::Albums] {
        for empty in [false, true] {
            let rows = if empty { vec![] } else { vec![row(kind, 1)] };
            let mut frames = start();
            frames.extend(scan(kind, &rows));
            frames.extend(scan(kind, &rows));
            let mut f = server(frames).await;
            store_account(&mut f.provider);
            let result = f
                .provider
                .native_purchases(kind, &request(Some("A"), 999))
                .await
                .unwrap();
            assert!(result.items.is_empty());
            assert_eq!(result.pagination.total, Some(u64::from(!empty)));
            assert!(!result.pagination.has_more);
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn purchase_changes_on_later_pages_and_reordered_or_unresolved_records_cannot_pass_first_page_only_checks()
 {
    for kind in [Kind::Tracks, Kind::Albums] {
        for case in ["later_metadata", "order", "total", "unresolved_identity"] {
            let rows = (1..=kind.page_size() + 1)
                .map(|id| row(kind, id as u64))
                .collect::<Vec<_>>();
            let mut changed = rows.clone();
            let last = changed.len() - 1;
            match case {
                "later_metadata" => {
                    changed[last][if kind == Kind::Tracks {
                        "songname"
                    } else {
                        "album_name"
                    }] = json!("Concurrent change")
                }
                "order" => changed.swap(0, 1),
                "total" => {
                    changed.pop();
                }
                _ => {
                    changed[last]
                        .as_object_mut()
                        .unwrap()
                        .remove(if kind == Kind::Tracks {
                            "album_audio_id"
                        } else {
                            "album_id"
                        });
                }
            }
            let mut frames = start();
            frames.extend(scan(kind, &rows));
            frames.extend(scan(kind, &changed));
            let count = frames.len();
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let p = f
                .provider
                .caller_scope(&read(&store, "A").caller().unwrap())
                .unwrap();
            let e = p
                .native_purchases(kind, &request(None, 0))
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict, "{kind:?}:{case}");
            assert!(p.take_response_credential().unwrap().is_none());
            assert_eq!(f.requests.await.unwrap().len(), count);
        }
    }
}

#[tokio::test]
async fn repeated_goods_short_pages_and_inconsistent_totals_fail_without_partial_library() {
    for case in ["repeat", "short", "total"] {
        let kind = Kind::Tracks;
        let rows = (1..=51).map(|id| row(kind, id)).collect::<Vec<_>>();
        let mut frames = start();
        frames.push(page(kind, rows[..50].to_vec(), 51).into());
        let tail = if case == "repeat" {
            vec![rows[0].clone()]
        } else if case == "short" {
            vec![]
        } else {
            vec![rows[50].clone(), row(kind, 52)]
        };
        frames.push(page(kind, tail, if case == "total" { 52 } else { 51 }).into());
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        assert!(
            f.provider
                .account_purchased_tracks(&request(Some("A"), 0))
                .await
                .is_err()
        );
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn purchase_failures_preserve_only_verified_rotations_and_never_retry_the_failed_page() {
    for kind in [Kind::Tracks, Kind::Albums] {
        for code in [
            ErrorCode::UpstreamError,
            ErrorCode::AuthenticationRequired,
            ErrorCode::Conflict,
        ] {
            let mut frames = start();
            frames.push(match code {
                ErrorCode::AuthenticationRequired => {
                    raw(json!({"status":0,"error_code":20017})).into()
                }
                ErrorCode::Conflict => reply(json!({"userid":999,"goods":[],"total":0})).into(),
                _ => raw(json!({"status":0,"error_code":500})).into(),
            });
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let p = f
                .provider
                .caller_scope(&read(&store, "A").caller().unwrap())
                .unwrap();
            let e = p
                .native_purchases(kind, &request(None, 0))
                .await
                .unwrap_err();
            assert_eq!(e.code, code);
            assert_eq!(
                p.take_response_credential().unwrap().is_some(),
                code == ErrorCode::UpstreamError
            );
            assert_eq!(f.requests.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn purchases_stop_on_relogin_at_each_page_boundary_without_reviving_the_old_account() {
    for boundary in [0, 1] {
        let rows = vec![row(Kind::Tracks, 1)];
        let mut frames = start();
        let (blocked, tx) = paused(page(Kind::Tracks, rows.clone(), 1));
        if boundary == 1 {
            frames.push(page(Kind::Tracks, rows, 1).into());
        }
        frames.push(blocked);
        let mut f = server(frames).await;
        let store = store_account(&mut f.provider);
        let p = f.provider.clone();
        let task =
            tokio::spawn(async move { p.account_purchased_tracks(&request(Some("A"), 0)).await });
        for _ in 0..3 + boundary {
            f.seen.recv().await.unwrap();
        }
        let fresh = credential("111", "replacement");
        store.put(&fresh.stored("A").unwrap()).unwrap();
        tx.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&store, "A"), fresh);
        assert_eq!(f.requests.await.unwrap().len(), 3 + boundary);
    }
}

#[tokio::test]
async fn invalid_purchase_windows_fail_before_network_and_missing_accounts_never_fall_back() {
    let f = server(vec![]).await;
    for (limit, offset) in [(0, 0), (101, 0), (10, u32::MAX)] {
        assert_eq!(
            f.provider
                .account_purchased_tracks(&PageRequest {
                    limit,
                    offset,
                    account: None
                })
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .account_purchased_albums(&request(Some("missing"), 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(f.requests.await.unwrap().is_empty());
}
