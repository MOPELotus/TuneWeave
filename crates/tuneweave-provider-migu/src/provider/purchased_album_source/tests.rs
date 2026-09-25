use super::*;
use crate::credential::MiguCredential;
use crate::provider::account_media::tests::{reply, server, setup};
use crate::provider::session::tests::{gated, profile, read, stored};

fn album(id: &str, digital: bool) -> serde_json::Value {
    json!({"contentId":id,"resourceType":if digital {"5"} else {"2003"},"title":"Purchased album"})
}
fn subscriptions(rows: &[serde_json::Value], more: bool, token: &str) -> String {
    reply(
        json!({"code":"000000","data":{"resources":rows,"hasNextPage":more}}),
        Some(token),
    )
}
fn metadata(digital: bool) -> String {
    let data = if digital {
        json!({"resourceType":"5","contentId":"77","title":"Digital album","totalCount":"1"})
    } else {
        json!({"resourceType":"2003","albumId":"77","title":"Ordinary album","totalCount":"3"})
    };
    reply(json!({"code":"000000","data":data}), None)
}
fn tracks(digital: bool, name: &str) -> String {
    let ids = if digital { vec![33] } else { vec![11, 22, 22] };
    let songs: Vec<_> = ids.iter().map(|id| json!({"resourceType":"2","contentId":id.to_string(),"songId":format!("song{id}"),"copyrightId":format!("copyright{id}"),"songName":name,"albumId":if digital {"444"} else {"77"},"album":"Original song album"})).collect();
    let mut data = json!({"songList":songs,"totalCount":ids.len()});
    if !digital {
        data["hasNext"] = json!(false);
    }
    reply(json!({"code":"000000","data":data}), None)
}
fn rows() -> Vec<serde_json::Value> {
    vec![album("77", false), album("77", true), album("77", false)]
}
fn frames(name: &str) -> Vec<String> {
    vec![
        profile("111", "pacmtoken: verified-0\r\n"),
        subscriptions(&rows(), false, "candidate-1"),
        profile("111", "pacmtoken: verified-1\r\n"),
        metadata(false),
        tracks(false, name),
        metadata(true),
        tracks(true, name),
        subscriptions(&rows(), false, "candidate-2"),
        profile("111", "pacmtoken: verified-2\r\n"),
    ]
}
fn request(alias: &str, offset: u32, limit: u32) -> PageRequest {
    PageRequest {
        limit,
        offset,
        account: Some(alias.into()),
    }
}

#[tokio::test]
async fn purchased_album_source_expands_typed_albums_preserves_duplicates_and_selected_ownership() {
    for mode in ["default", "named", "caller"] {
        let (mut provider, wire) = server(frames("Song")).await;
        let (store, original, alias) = setup(&mut provider, mode);
        let result = provider
            .playlist_source_items("111", SOURCE_TYPE, &request(alias, 2, 4))
            .await
            .unwrap();
        assert_eq!(result.pagination.total, Some(7));
        assert_eq!(result.pagination.next_offset, Some(6));
        assert_eq!(result.pagination.extensions["album_count"], 3);
        assert_eq!(result.pagination.extensions["unique_album_count"], 2);
        assert_eq!(result.pagination.extensions["catalogue_pages_fetched"], 2);
        for (index, (item, expected)) in result
            .items
            .iter()
            .zip(["22", "33", "11", "22"])
            .enumerate()
        {
            let PlaylistPlayableItem::Track(track) = item else {
                panic!("expected track")
            };
            assert_eq!(track.id, expected);
            assert_eq!(track.extensions["source_position"], index + 2);
            assert_eq!(track.extensions["source_user_id"], "111");
            assert_eq!(track.extensions["catalogue_scope"], "public");
            assert!(track.playable.is_none());
            if expected == "33" {
                assert_eq!(track.extensions["purchase_album_kind"], "digital_album");
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
        let exported = serde_json::to_string(&result).unwrap();
        for secret in ["verified-", "candidate-", "initial-pacm", "do-not-retain"] {
            assert!(!exported.contains(secret));
        }
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            assert_eq!(
                MiguCredential::parse_caller(
                    &provider.take_response_credential().unwrap().unwrap()
                )
                .unwrap()
                .token(),
                "verified-2"
            );
        } else {
            assert_eq!(read(&store, alias).token(), "verified-2");
            assert!(provider.take_response_credential().unwrap().is_none());
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        let wire = wire.await.unwrap();
        assert_eq!(wire.len(), 9);
        for message in &wire[3..7] {
            assert!(!message.contains("pacmtoken:"));
            assert!(!message.contains("cookie:"));
        }
        assert!(wire[3].starts_with("GET /MIGUM3.0/resource/album/v2.0?albumId=77 "));
        assert!(wire[5].starts_with("GET /MIGUM3.0/resource/dalbum/v2.0?dAlbumId=77 "));
        assert!(wire[7].contains("pacmtoken: verified-1\r\n"));
    }
}

#[tokio::test]
async fn purchased_album_source_header_and_windows_share_a_content_bound_snapshot() {
    let mut snapshots = Vec::new();
    for (name, header, offset) in [
        ("First", true, 0),
        ("First", false, 20),
        ("Changed", false, 0),
    ] {
        let (mut provider, wire) = server(frames(name)).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let extensions = if header {
            let source = provider
                .playlist_source("111", SOURCE_TYPE, Some(alias))
                .await
                .unwrap();
            assert_eq!(source.track_count, Some(7));
            assert_eq!(source.id, "111");
            source.extensions
        } else {
            let page = provider
                .playlist_source_items("111", SOURCE_TYPE, &request(alias, offset, 100))
                .await
                .unwrap();
            if offset > 0 {
                assert!(page.items.is_empty());
                assert!(!page.pagination.has_more);
            }
            page.pagination.extensions
        };
        snapshots.push(extensions["source_snapshot_id"].clone());
        wire.await.unwrap();
    }
    assert_eq!(snapshots[0], snapshots[1]);
    assert_ne!(snapshots[1], snapshots[2]);
}

#[tokio::test]
async fn purchased_album_source_empty_subscriptions_and_missing_purchase_titles_are_supported() {
    for empty in [true, false] {
        let mut responses = frames("Song");
        if empty {
            responses = vec![
                profile("111", ""),
                subscriptions(&[], false, "first-pacm"),
                profile("111", ""),
                subscriptions(&[], false, "last-pacm"),
                profile("111", ""),
            ];
        } else {
            let mut rows = rows();
            for row in &mut rows {
                row.as_object_mut().unwrap().remove("title");
            }
            responses[1] = subscriptions(&rows, false, "candidate-1");
            responses[7] = subscriptions(&rows, false, "candidate-2");
        }
        let (mut provider, wire) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let source = provider
            .playlist_source("111", SOURCE_TYPE, Some(alias))
            .await
            .unwrap();
        assert_eq!(source.track_count, Some(if empty { 0 } else { 7 }));
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn purchased_album_source_rejects_changed_subscriptions_identity_and_reflected_credentials() {
    for failure in ["subscription", "identity", "reflection"] {
        let mut responses = frames(if failure == "reflection" {
            "candidate-2"
        } else {
            "Song"
        });
        if failure == "subscription" {
            let mut changed = rows();
            changed[2]["contentId"] = json!("88");
            responses[7] = subscriptions(&changed, false, "candidate-2");
        } else if failure == "identity" {
            responses[8] = profile("222", "pacmtoken: wrong-identity\r\n");
        }
        let (mut provider, wire) = server(responses).await;
        let (store, _, alias) = setup(&mut provider, "named");
        let error = provider
            .playlist_source_items("111", SOURCE_TYPE, &request(alias, 0, 100))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            match failure {
                "subscription" => ErrorCode::Conflict,
                "identity" => ErrorCode::AuthenticationRequired,
                _ => ErrorCode::UpstreamError,
            }
        );
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        assert!(!error.message.contains("candidate-2"));
        assert!(
            !serde_json::to_string(&error.details)
                .unwrap()
                .contains("candidate-2")
        );
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn purchased_album_source_checks_generation_after_public_io_before_returning_its_error() {
    for failed_response in [false, true] {
        let mut responses = frames("Song");
        responses.truncate(4);
        if failed_response {
            responses[3] = reply(json!({"code":"999999"}), None);
        }
        let (mut provider, seen, release, wire) = gated(responses).await;
        let (store, _, alias) = setup(&mut provider, "named");
        let task = tokio::spawn(async move {
            provider
                .playlist_source("111", SOURCE_TYPE, Some(alias))
                .await
        });
        seen.await.unwrap();
        let replacement = MiguCredential::verified("111".into(), "verified-1".into()).unwrap();
        store.put(&stored(alias, &replacement)).unwrap();
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&store, alias), replacement);
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn purchased_album_source_rejects_bad_requests_before_network_and_incomplete_catalogues() {
    let (mut provider, wire) = server(Vec::new()).await;
    let (_, _, alias) = setup(&mut provider, "named");
    assert_eq!(
        provider
            .playlist_source("222", SOURCE_TYPE, Some(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    for (offset, limit) in [(0, 0), (0, 101), (u32::MAX, 2)] {
        assert_eq!(
            provider
                .playlist_source_items("111", SOURCE_TYPE, &request(alias, offset, limit))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(wire.await.unwrap().is_empty());
    let mut responses = frames("Song");
    responses.truncate(5);
    responses[4] = reply(
        json!({"code":"000000","data":{"songList":[],"totalCount":3,"hasNext":false}}),
        None,
    );
    let (mut provider, wire) = server(responses).await;
    let (_, _, alias) = setup(&mut provider, "named");
    assert_eq!(
        provider
            .playlist_source("111", SOURCE_TYPE, Some(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(wire.await.unwrap().len(), 5);
}

#[tokio::test]
async fn purchased_album_source_album_budget_is_not_a_partial_success() {
    let mut responses = vec![profile("111", "")];
    let rows: Vec<_> = (1..=129).map(|id| album(&id.to_string(), false)).collect();
    for (index, chunk) in rows.chunks(50).enumerate() {
        responses.push(subscriptions(chunk, index < 2, "rotated-pacm"));
        responses.push(profile("111", ""));
    }
    let (mut provider, wire) = server(responses).await;
    let (_, _, alias) = setup(&mut provider, "named");
    assert_eq!(
        provider
            .playlist_source("111", SOURCE_TYPE, Some(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(wire.await.unwrap().len(), 7);
}
