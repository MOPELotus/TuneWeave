use super::*;
use serde_json::Value;
use tuneweave_core::PlaylistPlayableItem;

const REF: &str = "legacy_web_collection:111:MQ";
const HASH: &str = "abcdef0123456789abcdef0123456789";

fn row(hash: &str, name: &str) -> Value {
    json!({"fileHash":hash,"fileName":name,"fileTimeLen":65000})
}
fn read_frames(rows: Value) -> Vec<Frame> {
    let mut frames = occurrence_frames();
    frames[3] = raw(rows);
    frames.into_iter().map(Frame::from).collect()
}
fn resolved(hash: &str, id: Value) -> Frame {
    raw(
        json!({"status":1,"err_code":0,"data":{"hash":hash,"album_audio_id":id,
        "song_name":"Resolved","timelength":65000,"userid":111}}),
    )
    .into()
}
fn page_request(account: Option<&str>, offset: u32, limit: u32) -> PageRequest {
    PageRequest {
        account: account.map(str::to_owned),
        offset,
        limit,
    }
}
fn track_ids(page: &Page<PlaylistPlayableItem>) -> Vec<String> {
    page.items
        .iter()
        .map(|item| match item {
            PlaylistPlayableItem::Track(track) => track.id.clone(),
            _ => panic!("unexpected non-track legacy source"),
        })
        .collect()
}

#[tokio::test]
async fn legacy_web_source_metadata_and_every_window_share_full_raw_fingerprint_for_all_owners() {
    for owner in ["default", "named", "caller"] {
        let rows = json!([
            row(HASH, "First"),
            row(&HASH.to_ascii_uppercase(), "Duplicate"),
            row(&"b".repeat(32), "Last")
        ]);
        let mut frames = read_frames(rows.clone());
        frames.extend(read_frames(rows.clone()));
        frames.push(resolved(HASH, json!(901)));
        frames.extend(read_frames(rows));
        frames.push(resolved(&"b".repeat(32), json!(902)));
        let mut fixture = server(frames).await;
        let store = Arc::new(Store::default());
        let original = source("111", "original-private-secret");
        let other = credential("999", "unrelated-native-secret");
        store.put(&other.stored("other").unwrap()).unwrap();
        fixture.provider.credential_store = Some(store.clone());
        let provider = if owner == "caller" {
            fixture
                .provider
                .caller_scope(&original.caller().unwrap())
                .unwrap()
        } else {
            store.put(&original.stored(owner).unwrap()).unwrap();
            fixture.provider.clone()
        };
        let account = (owner != "caller").then_some(owner);
        let metadata = provider
            .playlist_source(REF, "playlist", account)
            .await
            .unwrap();
        assert_eq!(metadata.id, REF);
        assert_eq!(metadata.track_count, Some(3));
        assert_eq!(metadata.extensions["library_owner_id"], "111");
        assert_eq!(
            metadata.extensions["snapshot_consistency"],
            "single_unversioned_response"
        );
        let first = provider
            .playlist_source_items(REF, "playlist", &page_request(account, 0, 2))
            .await
            .unwrap();
        let second = provider
            .playlist_source_items(REF, "playlist", &page_request(account, 2, 2))
            .await
            .unwrap();
        assert_eq!(track_ids(&first), vec!["901", "901"]);
        assert_eq!(track_ids(&second), vec!["902"]);
        assert!(first.pagination.has_more);
        assert_eq!(first.pagination.next_offset, Some(2));
        assert!(!second.pagination.has_more);
        for page in [&first, &second] {
            assert_eq!(page.pagination.total, Some(3));
            assert_eq!(
                page.pagination.extensions["source_snapshot_id"],
                metadata.extensions["source_snapshot_id"]
            );
            assert_eq!(page.pagination.extensions["complete_read"], true);
        }
        assert_eq!(read(&store, "other"), other);
        assert_eq!(
            provider.take_response_credential().unwrap().is_some(),
            owner == "caller"
        );
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), 17);
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /uc/getdata.php?type=17&"))
                .count(),
            3
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("GET /play/songinfo?"))
                .count(),
            2
        );
        assert!(
            requests
                .iter()
                .all(|r| !r.contains("unrelated-native-secret"))
        );
    }
}

#[tokio::test]
async fn legacy_web_source_empty_and_exhausted_track_pages_keep_complete_snapshot() {
    for rows in [json!([]), json!([row("opaque-cloud-file", "Not mapped")])] {
        let mut frames = read_frames(rows.clone());
        frames.extend(read_frames(rows.clone()));
        frames.extend(read_frames(rows.clone()));
        let fixture = server(frames).await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-private-secret").caller().unwrap())
            .unwrap();
        let metadata = provider
            .playlist_source(REF, "playlist", None)
            .await
            .unwrap();
        assert_eq!(
            metadata.track_count,
            Some(rows.as_array().unwrap().len() as u64)
        );
        let offset = if rows.as_array().unwrap().is_empty() {
            0
        } else {
            5
        };
        let tracks = provider
            .playlist_tracks(REF, &page_request(None, offset, 2))
            .await
            .unwrap();
        let items = provider
            .playlist_source_items(REF, "playlist", &page_request(None, offset, 2))
            .await
            .unwrap();
        assert!(tracks.items.is_empty());
        assert!(items.items.is_empty());
        assert!(!tracks.pagination.has_more);
        assert_eq!(tracks.pagination.next_offset, None);
        assert_eq!(
            tracks.pagination.extensions["source_snapshot_id"],
            metadata.extensions["source_snapshot_id"]
        );
        assert_eq!(
            items.pagination.extensions["source_snapshot_id"],
            metadata.extensions["source_snapshot_id"]
        );
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), 15);
        assert!(
            requests
                .iter()
                .all(|r| !r.starts_with("GET /play/songinfo?"))
        );
    }
}

#[tokio::test]
async fn legacy_web_source_track_pages_fail_on_unresolved_occurrences_instead_of_filtering() {
    for items_api in [false, true] {
        for opaque in [false, true] {
            let other = if opaque {
                "opaque-file".to_owned()
            } else {
                "b".repeat(32)
            };
            let rows = json!([row(HASH, "Known"), row(&other, "Unknown")]);
            let mut frames = read_frames(rows);
            frames.push(resolved(HASH, json!(901)));
            if !opaque {
                frames.push(resolved(&other, Value::Null));
            }
            let count = frames.len();
            let fixture = server(frames).await;
            let provider = fixture
                .provider
                .caller_scope(&source("111", "original-private-secret").caller().unwrap())
                .unwrap();
            let error = if items_api {
                provider
                    .playlist_source_items(REF, "playlist", &page_request(None, 0, 2))
                    .await
                    .unwrap_err()
            } else {
                provider
                    .playlist_tracks(REF, &page_request(None, 0, 2))
                    .await
                    .unwrap_err()
            };
            assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
            assert_eq!(error.details["unresolved_occurrence_position"], 1);
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(fixture.requests.await.unwrap().len(), count);
        }
    }
}

#[tokio::test]
async fn legacy_web_source_fingerprint_detects_changes_outside_the_returned_window() {
    for case in ["hash", "name", "duration", "order"] {
        let rows = json!([
            row(HASH, "First"),
            row(&"b".repeat(32), "Second"),
            row(&"c".repeat(32), "Third")
        ]);
        let mut changed = rows.clone();
        match case {
            "hash" => changed[2]["fileHash"] = json!("d".repeat(32)),
            "name" => changed[2]["fileName"] = json!("Changed"),
            "duration" => changed[2]["fileTimeLen"] = json!(66000),
            _ => changed.as_array_mut().unwrap().swap(1, 2),
        }
        let mut frames = read_frames(rows);
        frames.extend(read_frames(changed));
        frames.push(resolved(HASH, json!(901)));
        let fixture = server(frames).await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-private-secret").caller().unwrap())
            .unwrap();
        let metadata = provider
            .playlist_source(REF, "playlist", None)
            .await
            .unwrap();
        let page = provider
            .playlist_source_items(REF, "playlist", &page_request(None, 0, 1))
            .await
            .unwrap();
        assert_eq!(track_ids(&page), vec!["901"]);
        // The generic importer rejects this mismatch; no cross-request account
        // content is cached in the provider to conceal the changed response.
        assert_ne!(
            page.pagination.extensions["source_snapshot_id"],
            metadata.extensions["source_snapshot_id"],
            "{case}"
        );
        assert_eq!(fixture.requests.await.unwrap().len(), 11);
    }
}

#[tokio::test]
async fn legacy_web_source_rejects_wrong_uid_and_other_source_types_before_io() {
    let fixture = server(vec![]).await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-private-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        provider
            .playlist_source("legacy_web_collection:222:MQ", "playlist", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        provider
            .playlist_tracks("legacy_web_collection:222:MQ", &track_request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    for kind in [
        "favorite_tracks",
        "purchased_tracks",
        "purchased_albums",
        "unknown",
    ] {
        assert_eq!(
            provider
                .playlist_source(REF, kind, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
        assert_eq!(
            provider
                .playlist_source_items(REF, kind, &track_request(None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }
    assert!(fixture.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn legacy_web_source_metadata_requires_same_uid_after_type17_cookie_and_refresh() {
    for reflected_cookie in [false, true] {
        let rows = json!([row(HASH, "Known")]);
        let mut frames = read_frames(rows.clone());
        if reflected_cookie {
            frames[3] = cookie(rows, "222", "wrong-cookie-secret").into();
            frames.truncate(4);
        } else {
            frames[4] = exchange("222", "wrong-cookie-secret").into();
        }
        let count = frames.len();
        let fixture = server(frames).await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-private-secret").caller().unwrap())
            .unwrap();
        let mut error = provider
            .playlist_source(REF, "playlist", None)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(error.take_caller_credential_update().is_none());
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(fixture.requests.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn legacy_web_source_metadata_and_pages_discard_late_results_after_relogin() {
    for metadata in [false, true] {
        for failed in [false, true] {
            let mut frames = read_frames(json!([]));
            frames.pop();
            let (last, resume) = paused(if failed {
                raw(json!({"status":0,"error_code":20017}))
            } else {
                exchange("111", "last-verified-secret")
            });
            frames.push(last);
            let mut fixture = server(frames).await;
            let store = Arc::new(Store::default());
            store
                .put(
                    &source("111", "original-private-secret")
                        .stored("A")
                        .unwrap(),
                )
                .unwrap();
            fixture.provider.credential_store = Some(store.clone());
            let provider = fixture.provider.clone();
            let p = provider.clone();
            let task = tokio::spawn(async move {
                if metadata {
                    p.playlist_source(REF, "playlist", Some("A"))
                        .await
                        .map(|_| ())
                } else {
                    p.playlist_source_items(REF, "playlist", &page_request(Some("A"), 0, 1))
                        .await
                        .map(|_| ())
                }
            });
            for _ in 0..5 {
                fixture.seen.recv().await.unwrap();
            }
            let replacement = source("111", "replacement-login");
            store.put(&replacement.stored("A").unwrap()).unwrap();
            resume.send(()).unwrap();
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
            assert_eq!(read(&store, "A"), replacement);
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(fixture.requests.await.unwrap().len(), 5);
        }
    }
}
