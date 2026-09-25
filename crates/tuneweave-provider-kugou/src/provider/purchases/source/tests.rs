use super::super::tests::{request, row, scan, start};
use super::*;
use crate::KugouLoginClient;
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{credential, paused, read, reply, server};

fn complete(rows: &[serde_json::Value]) -> Vec<crate::provider::session::tests::Frame> {
    let mut frames = start();
    frames.extend(scan(Kind::Tracks, rows));
    frames.extend(scan(Kind::Tracks, rows));
    frames
}

#[tokio::test]
async fn purchased_source_keeps_owner_snapshot_order_and_repeated_tracks_for_native_clients() {
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for caller in [false, true] {
            let rows = (1..=55).map(|id| row(Kind::Tracks, id)).collect::<Vec<_>>();
            let mut frames = complete(&rows);
            frames.extend(complete(&rows));
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
            let original = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider
                    .caller_scope(&original.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let account = if caller { None } else { Some("A") };
            let metadata = provider
                .playlist_source("111", SOURCE, account)
                .await
                .unwrap();
            let page = provider
                .playlist_source_items("111", SOURCE, &request(account, 49))
                .await
                .unwrap();
            assert_eq!(metadata.resource_ref.id(), "111");
            assert_eq!(metadata.track_count, Some(55));
            assert_eq!(metadata.extensions["source_type"], SOURCE);
            assert_eq!(
                metadata.extensions["source_snapshot_id"],
                page.pagination.extensions["source_snapshot_id"]
            );
            assert_eq!(page.pagination.extensions["source_user_id"], "111");
            assert_eq!(page.pagination.extensions["upstream_pages_fetched"], 4);
            assert_eq!(page.pagination.total, Some(55));
            assert!(!page.pagination.has_more);
            assert_eq!(page.items.len(), 6);
            let tracks = page
                .items
                .iter()
                .map(|item| match item {
                    PlaylistPlayableItem::Track(track) => track,
                    _ => panic!("purchase must resolve to track"),
                })
                .collect::<Vec<_>>();
            assert_eq!(tracks[0].id, tracks[3].id);
            for (index, track) in tracks.iter().enumerate() {
                assert_eq!(track.extensions["goods_id"], (50 + index).to_string());
                assert_eq!(track.name, format!("Song {}", 50 + index));
                assert_eq!(track.playable, None);
                assert!(track.available_qualities.is_empty());
            }
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), original);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            let all = f.requests.await.unwrap();
            assert_eq!(all.len(), 12);
            for index in [2, 3, 4, 5, 8, 9, 10, 11] {
                assert!(all[index].starts_with("POST /openapi/copyright/v1/audio/get_goods?"));
                let body: serde_json::Value =
                    serde_json::from_str(all[index].split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(body["userid"], 111);
                assert_eq!(body["token"], "next");
            }
        }
    }
}

#[tokio::test]
async fn purchased_source_fails_on_unresolved_records_even_outside_the_requested_window() {
    for metadata in [false, true] {
        let mut rows = (1..=55).map(|id| row(Kind::Tracks, id)).collect::<Vec<_>>();
        rows[54].as_object_mut().unwrap().remove("album_audio_id");
        let mut f = server(complete(&rows)).await;
        store_account(&mut f.provider);
        let error = if metadata {
            f.provider
                .playlist_source("111", SOURCE, Some("A"))
                .await
                .err()
                .unwrap()
        } else {
            f.provider
                .playlist_source_items("111", SOURCE, &request(Some("A"), 0))
                .await
                .err()
                .unwrap()
        };
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["unresolved_entries"], 1);
        assert_eq!(f.requests.await.unwrap().len(), 6);
    }
}

#[tokio::test]
async fn purchased_source_rejects_other_owners_web_and_unknown_sources_before_network() {
    let mut f = server(vec![]).await;
    let store = store_account(&mut f.provider);
    for (uid, code) in [
        ("222", ErrorCode::PermissionDenied),
        ("0111", ErrorCode::InvalidRequest),
        ("", ErrorCode::InvalidRequest),
    ] {
        assert_eq!(
            f.provider
                .playlist_source(uid, SOURCE, Some("A"))
                .await
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(
            f.provider
                .playlist_source_items(uid, SOURCE, &request(Some("A"), 0))
                .await
                .unwrap_err()
                .code,
            code
        );
    }
    assert_eq!(
        f.provider
            .playlist_source("111", "unknown_purchase_source", Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    let web =
        KugouCredential::verified_web(crate::web::WebSession::test_session("111", "web-token"))
            .unwrap();
    store.put(&web.stored("A").unwrap()).unwrap();
    assert_eq!(
        f.provider
            .playlist_source("111", SOURCE, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn purchased_source_empty_and_outside_windows_are_confirmed_complete() {
    for rows in [vec![], vec![row(Kind::Tracks, 1)]] {
        let mut f = server(complete(&rows)).await;
        store_account(&mut f.provider);
        let page = f
            .provider
            .playlist_source_items("111", SOURCE, &request(Some("A"), 99))
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.pagination.total, Some(rows.len() as u64));
        assert_eq!(page.pagination.extensions["complete_read"], true);
        assert!(!page.pagination.has_more);
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn purchased_source_snapshot_changes_when_a_later_window_is_reordered() {
    let rows = (1..=55).map(|id| row(Kind::Tracks, id)).collect::<Vec<_>>();
    let mut changed = rows.clone();
    changed.swap(51, 52);
    let mut frames = complete(&rows);
    frames.extend(complete(&changed));
    let mut f = server(frames).await;
    store_account(&mut f.provider);
    let metadata = f
        .provider
        .playlist_source("111", SOURCE, Some("A"))
        .await
        .unwrap();
    let page = f
        .provider
        .playlist_source_items("111", SOURCE, &request(Some("A"), 0))
        .await
        .unwrap();
    // The core importer compares these IDs before appending any items.
    assert_ne!(
        metadata.extensions["source_snapshot_id"],
        page.pagination.extensions["source_snapshot_id"]
    );
    assert_eq!(f.requests.await.unwrap().len(), 12);
}

#[tokio::test]
async fn purchased_source_account_replacement_during_confirmation_is_not_rebound_to_the_new_login()
{
    let rows = vec![row(Kind::Tracks, 1)];
    let mut frames = start();
    frames.extend(scan(Kind::Tracks, &rows));
    let (last, resume) = paused(reply(
        json!({"userid":111,"goods":rows,"total":1,"pagesize":50}),
    ));
    frames.push(last);
    let mut f = server(frames).await;
    let store = store_account(&mut f.provider);
    let provider = f.provider.clone();
    let task =
        tokio::spawn(async move { provider.playlist_source("111", SOURCE, Some("A")).await });
    for _ in 0..4 {
        f.seen.recv().await.unwrap();
    }
    let next = credential("222", "different-login");
    store.put(&next.stored("A").unwrap()).unwrap();
    resume.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(read(&store, "A"), next);
    assert!(f.provider.take_response_credential().unwrap().is_none());
    assert_eq!(f.requests.await.unwrap().len(), 4);
}
