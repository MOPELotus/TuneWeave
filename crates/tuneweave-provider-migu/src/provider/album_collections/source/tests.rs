use super::*;
use crate::client::{album_collections::tests::entry, albums::AlbumKind};
use crate::credential::MiguCredential;
use crate::provider::{
    account_media::tests::{reply, server, setup},
    session::tests::{gated, profile, read, stored},
};

fn rows() -> Vec<serde_json::Value> {
    vec![
        entry(AlbumKind::Ordinary, "77"),
        entry(AlbumKind::Digital, "77"),
    ]
}
fn collection(rows: &[serde_json::Value], total: usize, token: &str) -> String {
    reply(
        json!({"code":"000000","collections":rows,"totalCount":total}),
        Some(token),
    )
}
fn metadata(id: &str, digital: bool, count: usize) -> String {
    let data = if digital {
        json!({"resourceType":"5","contentId":id,"title":"Digital","totalCount":count})
    } else {
        json!({"resourceType":"2003","albumId":id,"title":"Ordinary","totalCount":count})
    };
    reply(json!({"code":"000000","data":data}), None)
}
fn tracks(id: &str, ids: &[u32], total: usize, more: bool, name: &str) -> String {
    let songs: Vec<_> = ids.iter().map(|song| json!({"resourceType":"2","contentId":song.to_string(),"songId":format!("song{song}"),"copyrightId":format!("copyright{song}"),"songName":name,"albumId":id,"album":"Song album"})).collect();
    reply(
        json!({"code":"000000","data":{"songList":songs,"totalCount":total,"hasNext":more}}),
        None,
    )
}
fn frames(name: &str) -> Vec<String> {
    vec![
        profile("111", "pacmtoken: verified-0\r\n"),
        collection(&rows(), 2, "candidate-1"),
        profile("111", "pacmtoken: verified-1\r\n"),
        metadata("77", false, 3),
        tracks("77", &[11, 22, 22], 3, false, name),
        metadata("77", true, 2),
        tracks("444", &[22, 33], 2, false, name),
        collection(&rows(), 2, "candidate-2"),
        profile("111", "pacmtoken: verified-2\r\n"),
    ]
}
fn request(alias: &str, offset: u32, limit: u32) -> PageRequest {
    PageRequest {
        account: Some(alias.into()),
        offset,
        limit,
    }
}

#[tokio::test]
async fn favorite_album_source_preserves_mixed_identity_duplicates_and_all_credential_modes() {
    for mode in ["default", "named", "caller"] {
        let (mut p, wire) = server(frames("Song")).await;
        let (store, original, alias) = setup(&mut p, mode);
        let page = p
            .playlist_source_items("111", SOURCE_TYPE, &request(alias, 0, 100))
            .await
            .unwrap();
        assert_eq!(page.pagination.total, Some(5));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["album_count"], 2);
        assert_eq!(page.pagination.extensions["catalogue_pages_fetched"], 2);
        for (position, (item, expected)) in page
            .items
            .iter()
            .zip(["11", "22", "22", "22", "33"])
            .enumerate()
        {
            let PlaylistPlayableItem::Track(track) = item else {
                panic!("expected track")
            };
            assert_eq!(track.id, expected);
            assert_eq!(track.extensions["source_position"], position);
            assert_eq!(track.extensions["source_user_id"], "111");
            assert_eq!(track.extensions["favorite_album_id"], "77");
            assert_eq!(
                track.extensions["favorite_album_resource_type"],
                if position < 3 { "2003" } else { "5" }
            );
            assert_eq!(
                track.extensions["favorite_album_position"],
                usize::from(position >= 3)
            );
            assert_eq!(
                track.extensions["favorite_album_track_position"],
                if position < 3 { position } else { position - 3 }
            );
            assert!(track.playable.is_none());
            if position >= 3 {
                assert_eq!(
                    track
                        .album
                        .as_ref()
                        .unwrap()
                        .resource_ref
                        .as_ref()
                        .unwrap()
                        .to_string(),
                    "migu:444"
                );
            }
        }
        let exported = serde_json::to_string(&page).unwrap();
        for secret in ["verified-", "candidate-", "initial-pacm", "never-export"] {
            assert!(!exported.contains(secret));
        }
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            let update = p.take_response_credential().unwrap().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap().token(),
                "verified-2"
            );
            assert!(p.take_response_credential().unwrap().is_none());
        } else {
            assert_eq!(read(&store, alias).token(), "verified-2");
            assert!(p.take_response_credential().unwrap().is_none());
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        let wire = wire.await.unwrap();
        assert_eq!(wire.len(), 9);
        for index in [1, 7] {
            assert!(wire[index].starts_with("GET /pc/v1.0/user/collections.do?pageNo=1&pageSize=10&type=1&oPType=03&resourceType=2003%7C5 "));
        }
        assert!(wire[3].contains("/resource/album/v2.0?albumId=77 "));
        assert!(wire[5].contains("/resource/dalbum/v2.0?dAlbumId=77 "));
        for message in &wire[3..7] {
            assert!(!message.contains("pacmtoken:"));
            assert!(!message.contains("cookie:"));
        }
        assert!(wire.iter().all(|message| message.starts_with("GET ")));
    }
}

#[tokio::test]
async fn favorite_album_source_metadata_windows_and_changed_content_have_consistent_digests() {
    let mut snapshots = Vec::new();
    for (name, header, offset) in [
        ("First", true, 0),
        ("First", false, 1),
        ("First", false, 20),
        ("Changed", false, 0),
    ] {
        let (mut p, wire) = server(frames(name)).await;
        let (_, _, alias) = setup(&mut p, "named");
        let extensions = if header {
            let source = p
                .playlist_source("111", SOURCE_TYPE, Some(alias))
                .await
                .unwrap();
            assert_eq!(source.track_count, Some(5));
            source.extensions
        } else {
            let page = p
                .playlist_source_items("111", SOURCE_TYPE, &request(alias, offset, 2))
                .await
                .unwrap();
            assert_eq!(page.pagination.total, Some(5));
            assert_eq!(page.items.len(), if offset > 5 { 0 } else { 2 });
            assert_eq!(
                page.pagination.next_offset,
                (offset < 5).then_some(offset + 2)
            );
            page.pagination.extensions
        };
        assert_eq!(
            extensions["consistency"],
            "two_collection_reads_catalogue_once"
        );
        snapshots.push(extensions["source_snapshot_id"].clone());
        wire.await.unwrap();
    }
    assert_eq!(snapshots[0], snapshots[1]);
    assert_eq!(snapshots[1], snapshots[2]);
    assert_ne!(snapshots[2], snapshots[3]);
}

#[tokio::test]
async fn favorite_album_source_reads_all_collection_pages_and_accepts_empty_views() {
    for count in [0, 11] {
        let rows: Vec<_> = (1..=count)
            .map(|i| entry(AlbumKind::Ordinary, &i.to_string()))
            .collect();
        let mut replies = vec![profile("111", "")];
        let directory: Vec<_> = if rows.is_empty() {
            vec![collection(&[], 0, "rotation-pacm"), profile("111", "")]
        } else {
            rows.chunks(10)
                .flat_map(|chunk| {
                    [
                        collection(chunk, count, "rotation-pacm"),
                        profile("111", ""),
                    ]
                })
                .collect()
        };
        replies.extend(directory.clone());
        for i in 1..=count {
            replies.push(metadata(&i.to_string(), false, 0));
            replies.push(tracks(&i.to_string(), &[], 0, false, "unused"));
        }
        replies.extend(directory);
        let (mut p, wire) = server(replies).await;
        let (_, _, alias) = setup(&mut p, "named");
        let source = p
            .playlist_source("111", SOURCE_TYPE, Some(alias))
            .await
            .unwrap();
        assert_eq!(source.track_count, Some(0));
        assert_eq!(source.extensions["album_count"], count);
        let wire = wire.await.unwrap();
        assert_eq!(
            wire.iter()
                .filter(|r| r.contains("pageNo=2&pageSize=10"))
                .count(),
            if count == 0 { 0 } else { 2 }
        );
    }
}

#[tokio::test]
async fn favorite_album_source_rejects_invalid_selection_pagination_and_directory_pages() {
    let (mut p, wire) = server(vec![]).await;
    let (_, _, alias) = setup(&mut p, "named");
    for (id, expected) in [
        ("111?", ErrorCode::InvalidRequest),
        ("222", ErrorCode::PermissionDenied),
    ] {
        assert_eq!(
            p.playlist_source(id, SOURCE_TYPE, Some(alias))
                .await
                .unwrap_err()
                .code,
            expected
        );
    }
    for (offset, limit) in [(0, 0), (0, 101), (u32::MAX, 2)] {
        assert_eq!(
            p.playlist_source_items("111", SOURCE_TYPE, &request(alias, offset, limit))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(wire.await.unwrap().is_empty());
    for bad in [
        collection(&rows(), 3, "candidate-1"),
        collection(&[rows()[0].clone(), rows()[0].clone()], 2, "candidate-1"),
        reply(
            json!({"code":"000000","collections":[],"hasNext":true}),
            Some("candidate-1"),
        ),
    ] {
        let (mut p, wire) = server(vec![profile("111", ""), bad, profile("111", "")]).await;
        let (_, _, alias) = setup(&mut p, "named");
        assert_eq!(
            p.playlist_source("111", SOURCE_TYPE, Some(alias))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(wire.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn favorite_album_source_rejects_directory_drift_uid_changes_and_late_secret_reflection() {
    for failure in ["order", "metadata", "uid", "reflection"] {
        let mut replies = frames(if failure == "reflection" {
            "candidate-2"
        } else {
            "Song"
        });
        let mut changed = rows();
        match failure {
            "order" => changed.reverse(),
            "metadata" => changed[0]["title"] = json!("Changed"),
            "uid" => replies[8] = profile("222", ""),
            _ => {}
        }
        replies[7] = collection(&changed, 2, "candidate-2");
        let (mut p, wire) = server(replies).await;
        let (store, _, alias) = setup(&mut p, "caller");
        let failure_value = p
            .playlist_source_items("111", SOURCE_TYPE, &request(alias, 0, 1))
            .await
            .unwrap_err();
        assert_eq!(
            failure_value.code,
            match failure {
                "order" | "metadata" => ErrorCode::Conflict,
                "uid" => ErrorCode::AuthenticationRequired,
                _ => ErrorCode::UpstreamError,
            }
        );
        assert!(!failure_value.message.contains("candidate-2"));
        assert!(!failure_value.details.to_string().contains("candidate-2"));
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn favorite_album_source_never_delivers_partial_tracks_from_incomplete_later_albums() {
    for failure in ["ordinary", "digital", "wrong_album"] {
        let mut replies = frames("Song");
        let index = if failure == "ordinary" { 4 } else { 6 };
        replies[index] = tracks(
            "77",
            &[],
            if failure == "ordinary" { 3 } else { 2 },
            false,
            "Song",
        );
        if failure == "wrong_album" {
            replies[5] = metadata("88", true, 2);
            replies.truncate(6);
        } else {
            replies.truncate(index + 1);
        }
        let count = replies.len();
        let (mut p, wire) = server(replies).await;
        let (_, _, alias) = setup(&mut p, "named");
        assert_eq!(
            p.playlist_source_items("111", SOURCE_TYPE, &request(alias, 0, 1))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(wire.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn favorite_album_source_enforces_album_and_total_track_budgets() {
    let rows: Vec<_> = (1..=129)
        .map(|i| entry(AlbumKind::Ordinary, &i.to_string()))
        .collect();
    let mut replies = vec![profile("111", "")];
    for chunk in rows.chunks(10) {
        replies.extend([collection(chunk, 129, "rotation-pacm"), profile("111", "")]);
    }
    let (mut p, wire) = server(replies).await;
    let (_, _, alias) = setup(&mut p, "named");
    assert_eq!(
        p.playlist_source("111", SOURCE_TYPE, Some(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(wire.await.unwrap().len(), 27);

    let rows = [
        entry(AlbumKind::Ordinary, "77"),
        entry(AlbumKind::Ordinary, "88"),
    ];
    let mut replies = vec![
        profile("111", ""),
        collection(&rows, 2, "rotation-pacm"),
        profile("111", ""),
    ];
    for (id, count) in [("77", 5001), ("88", 5000)] {
        replies.push(metadata(id, false, count));
        let ids: Vec<_> = (1..=count as u32).collect();
        for (page, chunk) in ids.chunks(1000).enumerate() {
            replies.push(tracks(id, chunk, count, (page + 1) * 1000 < count, "Song"));
        }
    }
    let count = replies.len();
    let (mut p, wire) = server(replies).await;
    let (_, _, alias) = setup(&mut p, "named");
    assert_eq!(
        p.playlist_source("111", SOURCE_TYPE, Some(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(wire.await.unwrap().len(), count);
}

#[tokio::test]
async fn favorite_album_source_each_network_boundary_preserves_a_replacement_login() {
    for boundary in 0..9 {
        let mut replies = frames("Song");
        replies.truncate(boundary + 1);
        let (mut p, seen, release, wire) = gated(replies).await;
        let (store, _, alias) = setup(&mut p, "named");
        let task =
            tokio::spawn(async move { p.playlist_source("111", SOURCE_TYPE, Some(alias)).await });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .unwrap()
            .unwrap();
        let replacement =
            MiguCredential::verified("111".into(), "replacement-pacm".into()).unwrap();
        store.put(&stored(alias, &replacement)).unwrap();
        release.send(()).unwrap();
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict,
            "boundary {boundary}"
        );
        assert_eq!(read(&store, alias), replacement);
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn favorite_album_source_deadline_and_cancellation_keep_only_verified_caller_updates() {
    for cancel in [false, true] {
        for boundary in 0..9 {
            let mut replies = frames("Song");
            replies.truncate(boundary + 1);
            let (mut p, seen, _release, wire) = gated(replies).await;
            let (store, original, alias) = setup(&mut p, "caller");
            let p = Arc::new(p);
            let operation = p.clone();
            let task = tokio::spawn(async move {
                operation
                    .favorite_album_snapshot("111", Some(alias), Duration::from_millis(500))
                    .await
            });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            if cancel {
                task.abort();
                assert!(matches!(task.await, Err(error) if error.is_cancelled()));
                assert!(p.take_response_credential().unwrap().is_none());
            } else {
                let mut failure = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .err()
                    .unwrap();
                assert_eq!(failure.code, ErrorCode::UpstreamTimeout);
                let update = failure.take_caller_credential_update();
                if boundary == 0 {
                    assert!(update.is_none());
                } else {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        if boundary < 3 {
                            "verified-0"
                        } else {
                            "verified-1"
                        }
                    );
                }
            }
            assert_eq!(read(&store, alias), original);
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            wire.abort();
            assert!(wire.await.unwrap_err().is_cancelled());
        }
    }
}
