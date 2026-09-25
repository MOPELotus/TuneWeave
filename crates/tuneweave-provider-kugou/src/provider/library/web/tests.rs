use super::*;
use crate::provider::session::tests::{Frame, Store, credential, paused, raw, read, server};
use crate::web::WebSession;

mod enrichment;
mod source_read;

fn source(uid: &str, token: &str) -> KugouCredential {
    KugouCredential::verified_web(WebSession::test_session(uid, token)).unwrap()
}
fn cookie(body: serde_json::Value, uid: &str, token: &str) -> String {
    raw(body).replacen("Content-Type:", &format!(
        "Set-Cookie: KuGoo=KugooID={uid}&t={token}&a_id=1014&NickName=Listener; Domain=.kugou.com; Path=/\r\nContent-Type:"), 1)
}
fn exchange(uid: &str, token: &str) -> String {
    cookie(json!({"status":1,"error_code":0,"data":null}), uid, token)
}
fn directory() -> serde_json::Value {
    json!({"totalSize":5368709120_u64,"list":[
        {"listID":"9","listName":"Repeated name"},
        {"listID":"1","listName":"Repeated name"},
        {"listID":"x:2","listName":"Last"}
    ]})
}
fn frames() -> Vec<String> {
    vec![
        exchange("111", "first-verified-secret"),
        raw(directory()),
        exchange("111", "final-verified-secret"),
    ]
}
fn request(account: Option<&str>) -> PageRequest {
    PageRequest {
        account: account.map(str::to_owned),
        limit: 1,
        offset: 1,
    }
}

fn track_request(account: Option<&str>) -> PageRequest {
    PageRequest {
        account: account.map(str::to_owned),
        limit: 10,
        offset: 0,
    }
}

fn web_tracks() -> serde_json::Value {
    json!([
        {"fileHash":"same-hash","fileName":"First","fileTimeLen":"65000"},
        {"fileHash":"same-hash","fileName":"Second","fileTimeLen":78000},
        {"fileHash":"last-hash","fileName":"Third","fileTimeLen":91000}
    ])
}

fn occurrence_frames() -> Vec<String> {
    vec![
        exchange("111", "first-verified-secret"),
        raw(directory()),
        exchange("111", "directory-verified-secret"),
        raw(web_tracks()),
        exchange("111", "tracks-verified-secret"),
    ]
}

#[tokio::test]
async fn legacy_web_library_uses_selected_cookie_and_full_directory_in_all_ownership_sources() {
    for owner in ["default", "named", "caller"] {
        let mut fixture = server(frames().into_iter().map(Frame::from).collect()).await;
        let store = Arc::new(Store::default());
        let original = source("111", "original-private-secret");
        let other = credential("999", "other-native-secret");
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
        let result = provider.account_playlists(&request(account)).await.unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].extensions["legacy_list_id"], "1");
        assert_eq!(result.items[0].id, "legacy_web_collection:111:MQ");
        assert!(result.items[0].creator.is_none());
        assert!(result.items[0].track_count.is_none());
        assert_eq!(result.pagination.total, Some(3));
        assert_eq!(result.pagination.next_offset, Some(2));
        assert!(result.pagination.has_more);
        assert_eq!(
            result.pagination.extensions["storage_used_bytes"],
            5368709120_u64
        );
        assert_eq!(
            result.pagination.extensions["source"],
            "legacy_web_collection"
        );
        assert_eq!(read(&store, "other"), other);
        assert_eq!(
            provider.take_response_credential().unwrap().is_some(),
            owner == "caller"
        );
        if owner != "caller" {
            assert!(read(&store, owner).same_login(&original));
        }
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        let wire = &requests[1];
        assert!(wire.starts_with("POST /uc/getdata.php?type=16 "));
        assert!(wire.contains("t=first-verified-secret"));
        assert!(wire.contains("referer: https://www.kugou.com/uc/view/ikugou.html"));
        assert!(wire.contains("origin: https://www.kugou.com"));
        assert!(wire.contains("application/x-www-form-urlencoded"));
        let form = url::form_urlencoded::parse(wire.split_once("\r\n\r\n").unwrap().1.as_bytes())
            .into_owned()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(form.len(), 1);
        assert!((0.0..1.0).contains(&form["n"].parse::<f64>().unwrap()));
        assert!(!wire.contains("signature="));
        assert!(!wire.contains("userid="));
        assert!(
            !requests
                .iter()
                .any(|value| value.contains("other-native-secret"))
        );
        assert!(requests[2].starts_with("POST /v1/login_by_token_get?"));
    }
}

#[tokio::test]
async fn legacy_web_library_empty_exhausted_and_metadata_use_only_the_legacy_snapshot() {
    for (list, offset, count) in [(json!([]), 0, 0), (directory()["list"].clone(), 99, 3)] {
        let fixture = server(vec![
            exchange("111", "first-secret").into(),
            raw(json!({"totalSize":9,"list":list})).into(),
            exchange("111", "last-secret").into(),
        ])
        .await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-secret").caller().unwrap())
            .unwrap();
        let result = provider
            .account_playlists(&PageRequest {
                account: None,
                limit: 2,
                offset,
            })
            .await
            .unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.pagination.total, Some(count));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        assert_eq!(fixture.requests.await.unwrap().len(), 3);
    }
    let fixture = server(frames().into_iter().map(Frame::from).collect()).await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    let playlist = provider
        .playlist("legacy_web_collection:111:eDoy", None)
        .await
        .unwrap();
    assert_eq!(playlist.name, "Last");
    assert_eq!(playlist.extensions["legacy_list_id"], "x:2");
    assert_eq!(fixture.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn legacy_web_library_rejects_wrong_sources_pagination_and_unproved_operations_before_network()
 {
    let fixture = server(vec![]).await;
    let web = fixture
        .provider
        .caller_scope(&source("111", "private-secret").caller().unwrap())
        .unwrap();
    let native = fixture
        .provider
        .caller_scope(&credential("111", "native-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        web.playlist("legacy_web_collection:222:MQ", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        native
            .playlist("legacy_web_collection:111:MQ", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        fixture
            .provider
            .account_playlists(&request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert_eq!(
            web.account_playlists(&PageRequest {
                account: None,
                limit,
                offset
            })
            .await
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        web.account_playlists(&request(Some("other")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        web.user_created_playlists("111", &request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        web.favorite_playlist(None).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        native
            .playlist_tracks("legacy_web_collection:111:MQ", &request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        web.playlist_source("legacy_web_collection:111:MQ", "favorite_tracks", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(fixture.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn legacy_web_library_rejects_invalid_tail_failures_and_redirects_without_fallback() {
    let mut bad_tail = directory();
    bad_tail["list"][2]["listID"] = json!(null);
    for reply in [
        raw(bad_tail),
        raw(json!("fail")),
        raw(json!({"status":0,"message":"private-message","totalSize":0,"list":[]})),
        "HTTP/1.1 302 Found\r\nLocation: https://other.invalid/\r\nContent-Length: 0\r\n\r\n"
            .into(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\n\r\n<html></html>"
            .into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\n\r\n"
            .into(),
    ] {
        let fixture = server(vec![exchange("111", "first-secret").into(), reply.into()]).await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-secret").caller().unwrap())
            .unwrap();
        let error = provider
            .account_playlists(&request(None))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("private-message"));
        assert_eq!(fixture.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn legacy_web_library_requires_independent_same_uid_before_releasing_cookie_candidate() {
    for wrong in [false, true] {
        let fixture = server(vec![
            exchange("111", "first-secret").into(),
            cookie(directory(), "111", "candidate-secret").into(),
            exchange(if wrong { "222" } else { "111" }, "last-secret").into(),
        ])
        .await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-secret").caller().unwrap())
            .unwrap();
        let result = provider.account_playlists(&request(None)).await;
        if wrong {
            let mut error = result.unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(error.take_caller_credential_update().is_none());
            assert!(provider.take_response_credential().unwrap().is_none());
        } else {
            assert!(result.is_ok());
        }
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].contains("t=candidate-secret"));
    }
    let fixture = server(vec![
        exchange("111", "first-secret").into(),
        cookie(directory(), "222", "wrong-cookie-secret").into(),
    ])
    .await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        provider
            .account_playlists(&request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(fixture.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn legacy_web_library_keeps_cookie_domain_scope_and_strict_json_for_php_mime() {
    let login_host_only =
        exchange("111", "host-only-secret").replace("; Domain=.kugou.com; Path=/", "; Path=/");
    let fixture = server(vec![login_host_only.into()]).await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        provider
            .account_playlists(&request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(fixture.requests.await.unwrap().len(), 1);

    let fixture = server(vec![
        exchange("111", "first-secret").into(),
        raw(directory())
            .replace("application/json", "text/html; charset=utf-8")
            .into(),
        exchange("111", "last-secret").into(),
    ])
    .await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        provider
            .account_playlists(&request(None))
            .await
            .unwrap()
            .pagination
            .total,
        Some(3)
    );
    assert_eq!(fixture.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn legacy_web_library_each_async_boundary_rejects_late_success_and_error_after_relogin() {
    for boundary in 0..3 {
        for failure in [false, true] {
            let mut replies = frames();
            replies.truncate(boundary + 1);
            let last = replies.pop().unwrap();
            let (gate, release) = paused(if failure {
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into()
            } else {
                last
            });
            let mut replies = replies.into_iter().map(Frame::from).collect::<Vec<_>>();
            replies.push(gate);
            let mut fixture = server(replies).await;
            let store = Arc::new(Store::default());
            store
                .put(&source("111", "original-secret").stored("A").unwrap())
                .unwrap();
            fixture.provider.credential_store = Some(store.clone());
            let provider = fixture.provider.clone();
            let task =
                tokio::spawn(async move { provider.account_playlists(&request(Some("A"))).await });
            for _ in 0..=boundary {
                fixture.seen.recv().await.unwrap();
            }
            let replacement = source("111", "replacement-secret");
            store.put(&replacement.stored("A").unwrap()).unwrap();
            release.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(error.take_caller_credential_update().is_none());
            assert_eq!(read(&store, "A"), replacement);
            assert!(
                fixture
                    .provider
                    .take_response_credential()
                    .unwrap()
                    .is_none()
            );
            assert_eq!(fixture.requests.await.unwrap().len(), boundary + 1);
        }
    }
}

#[tokio::test]
async fn legacy_web_library_cancellation_timeout_and_secret_reflection_never_release_results() {
    for cancel in [true, false] {
        let (gate, release) = paused(raw(directory()));
        let mut fixture = server(vec![exchange("111", "first-secret").into(), gate]).await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-secret").caller().unwrap())
            .unwrap();
        let scoped = provider.clone();
        let task = tokio::spawn(async move {
            scoped
                .legacy_web_library_snapshot(None, None, Duration::from_secs(2))
                .await
        });
        for _ in 0..2 {
            fixture.seen.recv().await.unwrap();
        }
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(2)).await;
            assert_eq!(
                task.await.unwrap().unwrap_err().code,
                ErrorCode::UpstreamTimeout
            );
            tokio::time::resume();
        }
        assert!(provider.take_response_credential().unwrap().is_none());
        let _ = release.send(());
        fixture.requests.await.unwrap();
    }
    for field in ["listName", "listID"] {
        let mut body = directory();
        body["list"][0][field] = json!("first-secret");
        let fixture = server(vec![
            exchange("111", "first-secret").into(),
            raw(body).into(),
            exchange("111", "last-secret").into(),
        ])
        .await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-secret").caller().unwrap())
            .unwrap();
        assert_eq!(
            provider
                .account_playlists(&request(None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(fixture.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn legacy_web_track_occurrences_preserve_hash_duplicates_and_duration_for_each_owner() {
    for owner in ["default", "named", "caller"] {
        let mut fixture = server(occurrence_frames().into_iter().map(Frame::from).collect()).await;
        let store = Arc::new(Store::default());
        let original = source("111", "original-private-secret");
        let other = credential("999", "other-native-secret");
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
        let result = provider
            .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(account))
            .await
            .unwrap();
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.pagination.total, Some(3));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.extensions["complete_read"], true);
        assert_eq!(
            result.pagination.extensions["snapshot_consistency"],
            "single_unversioned_response"
        );
        assert_eq!(
            result.pagination.extensions["pagination_source"],
            "single_response_local_slice"
        );
        assert_eq!(result.items[0].position, 0);
        assert_eq!(result.items[0].extensions["file_hash"], "same-hash");
        assert_eq!(result.items[0].extensions["file_name"], "First");
        assert_eq!(result.items[0].extensions["duration_ms"], 65_000);
        assert_eq!(result.items[1].position, 1);
        assert_eq!(result.items[1].extensions["file_hash"], "same-hash");
        assert_eq!(result.items[1].extensions["file_name"], "Second");
        assert_eq!(result.items[1].extensions["duration_ms"], 78_000);
        assert!(result.items.iter().all(|item| item.track.is_none()));
        assert_eq!(
            result
                .items
                .iter()
                .map(|item| &item.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(read(&store, "other"), other);
        assert_eq!(
            provider.take_response_credential().unwrap().is_some(),
            owner == "caller"
        );
        if owner != "caller" {
            assert!(read(&store, owner).same_login(&original));
        }

        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), 5);
        assert!(requests[1].starts_with("POST /uc/getdata.php?type=16 "));
        assert!(requests[1].contains("t=first-verified-secret"));
        assert!(requests[3].starts_with("POST /uc/getdata.php?type=17&listid=1 "));
        assert!(requests[3].contains("t=directory-verified-secret"));
        assert!(requests[3].contains("referer: https://www.kugou.com/uc/view/ikugou.html"));
        assert!(requests[3].contains("origin: https://www.kugou.com"));
        assert!(requests[3].contains("application/x-www-form-urlencoded"));
        assert!(requests[4].starts_with("POST /v1/login_by_token_get?"));
        assert!(
            !requests
                .iter()
                .any(|wire| wire.contains("other-native-secret"))
        );
    }
}

#[tokio::test]
async fn legacy_web_track_occurrences_reject_wrong_owner_missing_lists_and_bad_responses() {
    let fixture = server(vec![]).await;
    let web = fixture
        .provider
        .caller_scope(&source("111", "private-secret").caller().unwrap())
        .unwrap();
    let native = fixture
        .provider
        .caller_scope(&credential("111", "native-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        web.playlist_track_occurrences("legacy_web_collection:222:MQ", &track_request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        native
            .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        web.playlist_track_occurrences(
            "legacy_web_collection:111:MQ",
            &PageRequest {
                account: None,
                limit: 0,
                offset: 0,
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    assert!(fixture.requests.await.unwrap().is_empty());

    let fixture = server(vec![
        exchange("111", "first-secret").into(),
        raw(json!({"totalSize":0,"list":[{"listID":"2","listName":"Other"}]})).into(),
    ])
    .await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        provider
            .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(fixture.requests.await.unwrap().len(), 2);

    for bad_tracks in [
        json!("fail"),
        json!({"data": []}),
        json!([
            {"fileHash":"h","fileName":"name","fileTimeLen":1},
            {"fileHash":"h","fileName":"malformed"}
        ]),
    ] {
        let fixture = server(vec![
            exchange("111", "first-secret").into(),
            raw(directory()).into(),
            exchange("111", "directory-secret").into(),
            cookie(bad_tracks, "111", "untrusted-response-secret").into(),
        ])
        .await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-secret").caller().unwrap())
            .unwrap();
        let mut error = provider
            .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(None))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        let update = error
            .take_caller_credential_update()
            .expect("preserve the last independently verified Web cookie");
        let KugouCredential::Web(accepted) = KugouCredential::parse_caller(&update).unwrap() else {
            unreachable!()
        };
        assert_eq!(accepted.session.media_token().unwrap(), "directory-secret");
        assert_ne!(
            accepted.session.media_token().unwrap(),
            "untrusted-response-secret"
        );
        assert_eq!(fixture.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn legacy_web_track_occurrences_reject_account_material_reflected_by_the_upstream() {
    let mut reflected_directory = directory();
    reflected_directory["list"][0]["listName"] = json!("first-secret");
    let fixture = server(vec![
        exchange("111", "first-secret").into(),
        raw(reflected_directory).into(),
    ])
    .await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        provider
            .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(fixture.requests.await.unwrap().len(), 2);

    let mut reflected_tracks = web_tracks();
    reflected_tracks[0]["fileName"] = json!("tracks-secret");
    let fixture = server(vec![
        exchange("111", "first-secret").into(),
        raw(directory()).into(),
        exchange("111", "directory-secret").into(),
        raw(reflected_tracks).into(),
        exchange("111", "tracks-secret").into(),
    ])
    .await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        provider
            .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(fixture.requests.await.unwrap().len(), 5);
}

#[tokio::test]
async fn legacy_web_track_occurrences_enforce_cookie_identity_and_same_account_generation() {
    let fixture = server(vec![
        exchange("111", "first-secret").into(),
        raw(directory()).into(),
        exchange("111", "directory-secret").into(),
        cookie(web_tracks(), "222", "wrong-cookie-secret").into(),
    ])
    .await;
    let provider = fixture
        .provider
        .caller_scope(&source("111", "original-secret").caller().unwrap())
        .unwrap();
    let mut error = provider
        .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(None))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(error.take_caller_credential_update().is_none());
    assert!(provider.take_response_credential().unwrap().is_none());
    assert_eq!(fixture.requests.await.unwrap().len(), 4);

    for boundary in 0..5 {
        let mut replies = occurrence_frames();
        replies.truncate(boundary + 1);
        let last = replies.pop().unwrap();
        let (gate, release) = paused(last);
        let mut replies = replies.into_iter().map(Frame::from).collect::<Vec<_>>();
        replies.push(gate);
        let mut fixture = server(replies).await;
        let store = Arc::new(Store::default());
        let original = source("111", "original-secret");
        store.put(&original.stored("A").unwrap()).unwrap();
        fixture.provider.credential_store = Some(store.clone());
        let provider = fixture.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .playlist_track_occurrences(
                    "legacy_web_collection:111:MQ",
                    &track_request(Some("A")),
                )
                .await
        });
        for _ in 0..=boundary {
            fixture.seen.recv().await.unwrap();
        }
        let replacement = source("111", "replacement-secret");
        store.put(&replacement.stored("A").unwrap()).unwrap();
        release.send(()).unwrap();
        let mut error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(error.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), replacement);
        assert!(
            fixture
                .provider
                .take_response_credential()
                .unwrap()
                .is_none()
        );
        assert_eq!(fixture.requests.await.unwrap().len(), boundary + 1);
    }
}
