use super::*;
use crate::client::account_playlist::tests::{home, metadata, page};
use crate::provider::{
    catalog::tests::server,
    session::tests::{Store, gated, profile, read, stored},
};
use std::time::Duration;
use tuneweave_core::ErrorCode;

fn reply(data: serde_json::Value, token: &str) -> String {
    let body = json!({"code":"000000","data":data}).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: {token}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn values(favorite: bool, ids: &[u32]) -> Vec<serde_json::Value> {
    let mut result = vec![];
    if favorite {
        result.push(home("77"));
    }
    result.push(metadata("77", "111", ids.len()));
    if ids.is_empty() {
        result.push(page(&[], 0));
    } else {
        for chunk in ids.chunks(50) {
            result.push(page(chunk, ids.len()));
        }
    }
    result.push(metadata("77", "111", ids.len()));
    if favorite {
        result.push(home("77"));
    }
    result
}
fn replies(values: Vec<serde_json::Value>) -> Vec<String> {
    let mut result = vec![profile("111", "pacmtoken: p1\r\n")];
    for (index, data) in values.into_iter().enumerate() {
        result.push(reply(data, &format!("candidate-{}", index + 2)));
        result.push(profile("111", &format!("pacmtoken: p{}\r\n", index + 2)));
    }
    result
}
fn setup(p: &mut MiguProvider) -> (Arc<Store>, MiguCredential, MiguCredential) {
    let s = Arc::new(Store::default());
    let a = MiguCredential::verified("111".into(), "initial".into()).unwrap();
    let b = MiguCredential::verified("222".into(), "other".into()).unwrap();
    s.put(&stored("A", &a)).unwrap();
    s.put(&stored("B", &b)).unwrap();
    p.credential_store = Some(s.clone());
    (s, a, b)
}
fn request(account: &str, limit: u32, offset: u32) -> PageRequest {
    PageRequest {
        account: Some(account.into()),
        limit,
        offset,
    }
}

#[tokio::test]
async fn account_playlist_foreign_response_uid_never_exports_or_commits_unverified_tokens() {
    for caller in [false, true] {
        for operation in 0..5 {
            let mut data = values(true, &[1, 2, 1]);
            data[operation]["userId"] = json!("222");
            data.truncate(operation + 1);
            let mut frames = replies(data);
            frames.pop();
            let count = frames.len();
            let (mut p, requests) = server(frames).await;
            let (store, a, b) = setup(&mut p);
            let alias = if caller {
                p = p.caller_scope(&a.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let mut failure = p.favorite_playlist(Some(alias)).await.unwrap_err();
            assert_eq!(failure.code, ErrorCode::AuthenticationRequired);
            assert!(failure.take_caller_credential_update().is_none());
            assert!(p.take_response_credential().unwrap().is_none());
            if caller {
                assert_eq!(read(&store, "A"), a);
            } else {
                assert!(
                    !store
                        .load_platform(Platform::Migu)
                        .unwrap()
                        .iter()
                        .any(|c| c.account == "A")
                );
            }
            assert_eq!(read(&store, "B"), b);
            assert_eq!(requests.await.unwrap().len(), count);
        }
    }
}

#[tokio::test]
async fn account_playlist_and_favorites_preserve_complete_order_scope_and_stable_content_fingerprint()
 {
    for favorite in [false, true] {
        let mut fingerprints = BTreeSet::new();
        for caller in [false, true] {
            for (offset, limit, expected) in
                [(0, 1, vec!["1"]), (49, 2, vec!["50", "1"]), (51, 3, vec![])]
            {
                let ids: Vec<_> = (1..=50).chain([1]).collect();
                let frames = replies(values(favorite, &ids));
                let count = frames.len();
                let (mut p, requests) = server(frames).await;
                let (store, a, b) = setup(&mut p);
                let alias = if caller {
                    p = p.caller_scope(&a.caller().unwrap()).unwrap();
                    "default"
                } else {
                    "A"
                };
                let result = if favorite {
                    p.user_favorite_tracks("111", &request(alias, limit, offset))
                        .await
                } else {
                    p.playlist_tracks("77", &request(alias, limit, offset))
                        .await
                }
                .unwrap();
                assert_eq!(
                    result
                        .items
                        .iter()
                        .map(|t| t.id.as_str())
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(result.pagination.total, Some(51));
                if offset == 49 {
                    assert_eq!(result.items[1].extensions["playlist_position"], 50);
                }
                fingerprints.insert(
                    result.pagination.extensions["source_snapshot_id"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
                assert!(
                    !serde_json::to_string(&result)
                        .unwrap()
                        .contains("candidate")
                );
                let update = p.take_response_credential().unwrap();
                if caller {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        format!("p{}", count.div_ceil(2))
                    );
                    assert_eq!(read(&store, "A"), a);
                } else {
                    assert!(update.is_none());
                    assert_eq!(read(&store, "A").token(), format!("p{}", count.div_ceil(2)));
                }
                assert_eq!(read(&store, "B"), b);
                let requests = requests.await.unwrap();
                assert_eq!(requests.len(), count);
                for (index, r) in requests.iter().enumerate() {
                    assert!(!r.contains("cookie:"));
                    assert!(!r.contains("do-not-export"));
                    assert_eq!(r.matches("\r\nreferer:").count(), 1);
                    let token = if index == 0 {
                        "initial".to_owned()
                    } else if index % 2 == 1 {
                        format!("p{}", index.div_ceil(2))
                    } else {
                        format!("candidate-{}", index / 2 + 1)
                    };
                    assert!(r.contains(&format!("pacmtoken: {token}\r\n")), "{index}");
                    if index % 2 == 1 {
                        assert!(r.contains("channel: 014X031\r\n"));
                        assert!(r.contains("deviceid:"));
                    }
                }
                let tracks: Vec<_> = requests
                    .iter()
                    .filter(|r| r.starts_with("GET /MIGUM3.0/resource/playlist/song/v2.0?"))
                    .collect();
                assert_eq!(tracks.len(), 2);
                assert!(tracks[0].contains("pageNo=1&pageSize=50&playlistId=77"));
                assert!(tracks[1].contains("pageNo=2&pageSize=50&playlistId=77"));
            }
        }
        assert_eq!(fingerprints.len(), 1);
    }
}

#[tokio::test]
async fn account_playlist_metadata_favorites_and_uni_sources_use_actual_identity_and_change_fingerprint_with_order()
 {
    let mut fingerprints = vec![];
    for ids in [vec![1, 2, 1], vec![2, 1, 1]] {
        let (mut p, requests) = server(replies(values(true, &ids))).await;
        setup(&mut p);
        let playlist = p
            .playlist_source("111", "favorite_tracks", Some("A"))
            .await
            .unwrap();
        assert_eq!(playlist.id, "77");
        assert_eq!(playlist.name, "Actual playlist name");
        assert_eq!(playlist.track_count, Some(3));
        assert_eq!(playlist.creator.unwrap().resource_ref, None);
        assert_eq!(playlist.extensions["source_type"], "favorite_tracks");
        assert!(
            !serde_json::to_string(&playlist.extensions)
                .unwrap()
                .contains("do-not-export")
        );
        fingerprints.push(playlist.extensions["source_snapshot_id"].clone());
        requests.await.unwrap();
    }
    assert_ne!(fingerprints[0], fingerprints[1]);
    for favorite in [false, true] {
        let (mut p, requests) = server(replies(values(favorite, &[]))).await;
        setup(&mut p);
        let page = p
            .playlist_source_items(
                if favorite { "111" } else { "77" },
                if favorite {
                    "favorite_tracks"
                } else {
                    "playlist"
                },
                &request("A", 10, 0),
            )
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.pagination.total, Some(0));
        requests.await.unwrap();
    }
}

#[tokio::test]
async fn account_playlist_rejects_bad_scope_missing_accounts_and_invalid_windows_before_network() {
    let (mut p, requests) = server(vec![]).await;
    let (_, a, _) = setup(&mut p);
    for (id, alias, code) in [
        ("0", "A", ErrorCode::InvalidRequest),
        ("077", "A", ErrorCode::InvalidRequest),
        ("77", "missing", ErrorCode::AuthenticationRequired),
    ] {
        assert_eq!(p.playlist(id, Some(alias)).await.unwrap_err().code, code);
    }
    assert_eq!(
        p.user_favorite_playlist("222", Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    for (limit, offset) in [(0, 0), (101, 0), (2, u32::MAX)] {
        assert_eq!(
            p.favorite_tracks(&request("A", limit, offset))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let caller = p.caller_scope(&a.caller().unwrap()).unwrap();
    assert_eq!(
        caller.favorite_playlist(Some("A")).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn account_playlist_failed_pages_final_metadata_and_favorite_identity_preserve_only_verified_updates()
 {
    for variant in 0..7 {
        let mut data = values(true, &[1, 2, 1]);
        let stop = match variant {
            0 => {
                data[1]["ownerId"] = json!("222");
                1
            }
            1 => {
                data[2]["playlistId"] = json!("88");
                2
            }
            2 => {
                data[2]["songList"] = json!([]);
                2
            }
            3 => {
                data[2]["totalCount"] = json!(4);
                2
            }
            4 => {
                data[3]["musicNum"] = json!(4);
                3
            }
            5 => {
                data[4] = home("88");
                4
            }
            _ => {
                data[3]["ownerId"] = json!("222");
                3
            }
        };
        data.truncate(stop + 1);
        let frames = replies(data);
        let count = frames.len();
        let (mut p, requests) = server(frames).await;
        let (_, a, _) = setup(&mut p);
        let p = p.caller_scope(&a.caller().unwrap()).unwrap();
        let mut e = p
            .favorite_tracks(&request("default", 1, 0))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError, "variant{variant}");
        assert_eq!(
            MiguCredential::parse_caller(&e.take_caller_credential_update().unwrap())
                .unwrap()
                .token(),
            format!("p{}", count.div_ceil(2))
        );
        assert_eq!(requests.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn account_playlist_each_await_rejects_relogin_and_logout() {
    let ids: Vec<_> = (1..=51).collect();
    let frames = replies(values(true, &ids));
    for remove in [false, true] {
        for stage in 1..=frames.len() {
            let (mut p, seen, release, server) = gated(frames[..stage].to_vec()).await;
            let (store, _, b) = setup(&mut p);
            let task = tokio::spawn(async move { p.favorite_tracks(&request("A", 10, 0)).await });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            let replacement = MiguCredential::verified("333".into(), "replacement".into()).unwrap();
            if remove {
                store.remove(Platform::Migu, "A").unwrap();
            } else {
                store.put(&stored("A", &replacement)).unwrap();
            }
            release.send(()).unwrap();
            let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict, "stage{stage}");
            assert!(e.take_caller_credential_update().is_none());
            if remove {
                assert!(
                    !store
                        .load_platform(Platform::Migu)
                        .unwrap()
                        .iter()
                        .any(|c| c.account == "A")
                );
            } else {
                assert_eq!(read(&store, "A"), replacement);
            }
            assert_eq!(read(&store, "B"), b);
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn account_playlist_each_timeout_preserves_the_last_profile_verified_token() {
    let ids: Vec<_> = (1..=51).collect();
    let frames = replies(values(true, &ids));
    for stage in 1..=frames.len() {
        let (mut p, seen, release, server) = gated(frames[..stage].to_vec()).await;
        let (_, a, _) = setup(&mut p);
        p.client = p
            .client
            .with_session_test_timeout(Duration::from_millis(300));
        let p = Arc::new(p.caller_scope(&a.caller().unwrap()).unwrap());
        let worker = p.clone();
        let task =
            tokio::spawn(async move { worker.favorite_tracks(&request("default", 10, 0)).await });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .unwrap()
            .unwrap();
        let mut e = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamTimeout);
        let expected = (stage > 1).then(|| format!("p{}", stage / 2));
        assert_eq!(
            e.take_caller_credential_update()
                .map(|c| MiguCredential::parse_caller(&c).unwrap().token().to_owned()),
            expected
        );
        assert_eq!(
            p.take_response_credential()
                .unwrap()
                .map(|c| MiguCredential::parse_caller(&c).unwrap().token().to_owned()),
            expected
        );
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        drop(release);
    }
}
