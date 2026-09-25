use super::*;
use crate::credential::MiguCredential;
use crate::provider::account_media::tests::{reply, resource, server, setup};
use crate::provider::session::tests::{gated, profile, read, stored};
use tuneweave_core::PlaylistPlayableItem;

fn ids(values: serde_json::Value, token: &str) -> String {
    let body = json!({"code":"000000","data":values}).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nSet-Cookie: pacmtoken={token}; Path=/; Secure; HttpOnly\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn frames(values: &[&str], selected: &[&str]) -> Vec<String> {
    let mut frames = vec![
        profile("111", "pacmtoken: verified-1\r\n"),
        ids(json!(values), "candidate-2"),
        profile("111", "pacmtoken: verified-2\r\n"),
    ];
    frames.extend(
        selected.iter().map(|id| {
            resource().replace("\"contentId\":\"123\"", &format!("\"contentId\":\"{id}\""))
        }),
    );
    frames.extend([
        ids(json!(values), "candidate-3"),
        profile("111", "pacmtoken: verified-3\r\n"),
    ]);
    frames
}
fn request(alias: &str) -> PageRequest {
    PageRequest {
        limit: 2,
        offset: 1,
        account: Some(alias.into()),
    }
}

#[tokio::test]
async fn purchases_selected_accounts_use_cookie_transport_and_preserve_complete_order() {
    for mode in ["default", "named", "caller"] {
        let (mut provider, wire) =
            server(frames(&["123", "124", "123", "125"], &["124", "123"])).await;
        let (store, original, alias) = setup(&mut provider, mode);
        let page = provider
            .account_purchased_tracks(&request(alias))
            .await
            .unwrap();
        assert_eq!(page.pagination.total, Some(4));
        assert_eq!(page.pagination.next_offset, Some(3));
        assert!(page.pagination.has_more);
        assert_eq!(
            page.pagination.extensions["consistency"],
            "two_complete_reads"
        );
        assert_eq!(page.pagination.extensions["source_user_id"], "111");
        assert_eq!(
            page.items
                .iter()
                .map(|i| i.track.as_ref().unwrap().id.as_str())
                .collect::<Vec<_>>(),
            ["124", "123"]
        );
        for (i, item) in page.items.iter().enumerate() {
            assert_eq!(item.extensions["source_position"], i + 1);
            assert_eq!(item.track.as_ref().unwrap().playable, None);
            assert_eq!(
                item.track.as_ref().unwrap().extensions["catalogue_scope"],
                "public"
            );
        }
        let output = serde_json::to_string(&page).unwrap();
        for forbidden in ["initial-pacm", "candidate-", "verified-", "do-not-retain"] {
            assert!(!output.contains(forbidden));
        }
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            assert_eq!(
                MiguCredential::parse_caller(
                    &provider.take_response_credential().unwrap().unwrap()
                )
                .unwrap()
                .token(),
                "verified-3"
            );
        } else {
            assert_eq!(read(&store, alias).token(), "verified-3");
            assert!(provider.take_response_credential().unwrap().is_none());
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        let wire = wire.await.unwrap();
        assert_eq!(wire.len(), 7);
        for (index, token) in [(1, "verified-1"), (5, "verified-2")] {
            let request = wire[index].to_ascii_lowercase();
            assert!(request.starts_with("get /migum3.0/strategy/song-ordered/v2.0 http/1.1\r\n"));
            assert!(request.contains(&format!("cookie: pacmtoken={token}\r\n")));
            assert!(request.contains("channel: 014x031\r\n"));
            assert!(!request.contains("\r\npacmtoken:"));
            assert!(!request.contains("\r\ntoken:"));
        }
        for index in [3, 4] {
            assert!(!wire[index].to_lowercase().contains("pacmtoken"));
        }
        assert!(wire[2].contains("pacmtoken: candidate-2\r\n"));
        assert!(wire[6].contains("pacmtoken: candidate-3\r\n"));
    }
}

#[tokio::test]
async fn purchased_tracks_expose_a_uid_scoped_uni_source_and_reject_unresolved_items() {
    let (mut provider, wire) = server(frames(&["123", "124"], &["123"])).await;
    let (_, _, alias) = setup(&mut provider, "named");
    let source = provider
        .playlist_source("111", "purchased_tracks", Some(alias))
        .await
        .unwrap();
    assert_eq!(source.id, "111");
    assert_eq!(source.resource_ref.to_string(), "migu:111");
    assert_eq!(source.track_count, Some(2));
    assert_eq!(source.extensions["source_type"], "purchased_tracks");
    assert_eq!(source.extensions["source_user_id"], "111");
    assert_eq!(source.extensions["complete_read"], true);
    assert_eq!(wire.await.unwrap().len(), 6);

    let (mut provider, wire) = server(frames(&["123", "124"], &["123", "124"])).await;
    let (_, _, alias) = setup(&mut provider, "named");
    let mut source_request = request(alias);
    source_request.offset = 0;
    let page = provider
        .playlist_source_items("111", "purchased_tracks", &source_request)
        .await
        .unwrap();
    assert_eq!(page.pagination.total, Some(2));
    assert_eq!(page.items.len(), 2);
    let ids = page
        .items
        .iter()
        .map(|item| match item {
            PlaylistPlayableItem::Track(track) => track.id.as_str(),
            _ => panic!("purchased source returned a non-track item"),
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, ["123", "124"]);
    assert_eq!(
        page.pagination.extensions["consistency"],
        "two_complete_reads"
    );
    assert_eq!(wire.await.unwrap().len(), 7);

    let mut unresolved = frames(&["123", "124"], &["123", "124"]);
    unresolved[3] = reply(json!({"code":"000000","resource":[]}), None);
    let (mut provider, wire) = server(unresolved).await;
    let (_, _, _alias) = setup(&mut provider, "named");
    let result = provider
        .playlist_source_items("111", "purchased_tracks", &source_request)
        .await;
    assert_eq!(result.unwrap_err().code, ErrorCode::UpstreamError);
    assert_eq!(wire.await.unwrap().len(), 7);

    let (mut provider, wire) = server(Vec::new()).await;
    let (_, _, alias) = setup(&mut provider, "named");
    let result = provider
        .playlist_source("222", "purchased_tracks", Some(alias))
        .await;
    assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
    assert!(wire.await.unwrap().is_empty());
}

#[tokio::test]
async fn purchases_empty_and_out_of_range_windows_still_verify_both_complete_lists() {
    for values in [vec![], vec!["123", "123"]] {
        let (mut provider, wire) = server(frames(&values, &[])).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let mut request = request(alias);
        request.offset = 100;
        let page = provider.account_purchased_tracks(&request).await.unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.pagination.total, Some(values.len() as u64));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(wire.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn purchases_missing_catalogue_keeps_identity_but_other_failures_and_late_list_changes_fail()
{
    for kind in [
        "missing",
        "malformed",
        "network",
        "changed-outside-window",
        "reordered",
        "duplicate-count",
    ] {
        let mut responses = frames(&["123", "124", "125"], &["124", "125"]);
        let expected = match kind {
            "missing" => {
                responses[3] = reply(json!({"code":"000000","resource":[]}), None);
                None
            }
            "malformed" => {
                responses.truncate(4);
                responses[3] = reply(json!({"code":"000000","resource":[{}]}), None);
                Some(ErrorCode::UpstreamError)
            }
            "network" => {
                responses.truncate(4);
                responses[3] =
                    "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .into();
                Some(ErrorCode::UpstreamError)
            }
            "changed-outside-window" => {
                responses[5] = ids(json!(["999", "124", "125"]), "candidate-3");
                Some(ErrorCode::Conflict)
            }
            "reordered" => {
                responses[5] = ids(json!(["123", "125", "124"]), "candidate-3");
                Some(ErrorCode::Conflict)
            }
            _ => {
                responses[5] = ids(json!(["123", "124", "125", "125"]), "candidate-3");
                Some(ErrorCode::Conflict)
            }
        };
        let (mut provider, wire) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "caller");
        let result = provider.account_purchased_tracks(&request(alias)).await;
        if let Some(expected) = expected {
            let mut failure = result.unwrap_err();
            assert_eq!(failure.code, expected, "{kind}");
            if expected == ErrorCode::Conflict {
                assert!(failure.take_caller_credential_update().is_none());
                assert!(provider.take_response_credential().unwrap().is_none());
            }
        } else {
            let page = result.unwrap();
            assert!(page.items[0].track.is_none());
            assert!(page.items[0].name.is_none());
            assert_eq!(page.items[0].extensions["resource_ref"], "migu:124");
            assert_eq!(page.items[0].extensions["catalogue_resolved"], false);
            assert_eq!(page.pagination.total, Some(3));
        }
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn purchases_protocol_failures_never_become_empty_success_or_accept_unverified_credentials() {
    for mode in ["named", "caller"] {
        for kind in [
            "bad-item",
            "missing-data",
            "null-data",
            "wrong-uid",
            "root-uid",
            "unauthorized",
            "forbidden",
            "rate-limit",
            "business",
            "oversize",
            "html",
            "cookie-conflict",
            "credential-reflection",
        ] {
            let mut responses = frames(&["123", "124", "125"], &["124", "125"]);
            responses.truncate(2);
            let (expected, verified) = match kind {
                "bad-item" => {
                    responses[1] = ids(json!([123]), "candidate-2");
                    responses.push(profile("111", "pacmtoken: verified-2\r\n"));
                    (ErrorCode::UpstreamError, "verified-2")
                }
                "null-data" => {
                    responses[1] = ids(json!(null), "candidate-2");
                    (ErrorCode::UpstreamError, "verified-1")
                }
                "missing-data" => {
                    responses[1] = reply(json!({"code":"000000"}), Some("untrusted"));
                    (ErrorCode::UpstreamError, "verified-1")
                }
                "wrong-uid" => {
                    responses.push(profile("222", "pacmtoken: untrusted\r\n"));
                    (ErrorCode::AuthenticationRequired, "")
                }
                "root-uid" => {
                    responses[1] = reply(
                        json!({"code":"000000","userId":"222","data":[]}),
                        Some("untrusted"),
                    );
                    (ErrorCode::AuthenticationRequired, "")
                }
                "unauthorized" => {
                    responses[1] = "HTTP/1.1 401 Unauthorized\r\npacmtoken: untrusted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::AuthenticationRequired, "")
                }
                "forbidden" => {
                    responses[1] =
                        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .into();
                    (ErrorCode::PermissionDenied, "verified-1")
                }
                "rate-limit" => {
                    responses[1] = "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::RateLimited, "verified-1")
                }
                "business" => {
                    responses[1] = reply(
                        json!({"code":"299999","info":"private-error","data":[]}),
                        Some("untrusted"),
                    );
                    (ErrorCode::UpstreamError, "verified-1")
                }
                "oversize" => {
                    responses[1] = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::UpstreamError, "verified-1")
                }
                "html" => {
                    responses[1] = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::UpstreamError, "verified-1")
                }
                "credential-reflection" => {
                    responses[0] = profile("111", "pacmtoken: alnumtoken\r\n");
                    responses[1] = ids(json!(["alnumtoken"]), "candidate-2");
                    responses.push(profile("111", "pacmtoken: verified-2\r\n"));
                    (ErrorCode::UpstreamError, "verified-2")
                }
                _ => {
                    responses[1] = responses[1].replace(
                        "Connection: close",
                        "pacmtoken: conflicting\r\nConnection: close",
                    );
                    (ErrorCode::UpstreamError, "verified-1")
                }
            };
            let (mut provider, wire) = server(responses).await;
            let (store, original, alias) = setup(&mut provider, mode);
            let mut failure = provider
                .account_purchased_tracks(&request(alias))
                .await
                .unwrap_err();
            assert_eq!(failure.code, expected, "{mode}/{kind}");
            assert!(!format!("{failure:?}").contains("private-error"));
            let update = failure.take_caller_credential_update();
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                if verified.is_empty() {
                    assert!(update.is_none());
                    assert!(provider.take_response_credential().unwrap().is_none());
                } else {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        verified
                    );
                }
            } else if verified.is_empty() {
                assert!(
                    !store
                        .load_platform(Platform::Migu)
                        .unwrap()
                        .iter()
                        .any(|v| v.account == alias)
                );
            } else {
                assert_eq!(read(&store, alias).token(), verified);
            }
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            wire.await.unwrap();
        }
    }
}

#[tokio::test]
async fn purchases_every_network_boundary_rejects_late_success_and_errors_after_source_changes() {
    for mode in ["default", "named", "caller"] {
        for boundary in 0..7 {
            for action in ["logout", "same-token-login", "switch-user"] {
                if mode == "caller" && action == "logout" {
                    continue;
                }
                for late_error in [false, true] {
                    let mut responses = frames(&["123", "124", "125"], &["124", "125"]);
                    responses.truncate(boundary + 1);
                    if late_error {
                        responses[boundary] = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    }
                    let (mut provider, seen, release, wire) = gated(responses).await;
                    let (store, _, alias) = setup(&mut provider, mode);
                    let provider = Arc::new(provider);
                    let operation = provider.clone();
                    let task = tokio::spawn(async move {
                        operation.account_purchased_tracks(&request(alias)).await
                    });
                    tokio::time::timeout(Duration::from_secs(5), seen)
                        .await
                        .unwrap()
                        .unwrap();
                    let token = if mode == "caller" {
                        provider
                            .caller_credential
                            .as_ref()
                            .unwrap()
                            .lock()
                            .unwrap()
                            .token()
                            .to_owned()
                    } else {
                        read(&store, alias).token().to_owned()
                    };
                    let replacement = MiguCredential::verified(
                        if action == "switch-user" {
                            "333"
                        } else {
                            "111"
                        }
                        .into(),
                        token,
                    )
                    .unwrap();
                    if mode == "caller" {
                        *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                            replacement.clone();
                    } else if action == "logout" {
                        store.remove(Platform::Migu, alias).unwrap();
                    } else {
                        store.put(&stored(alias, &replacement)).unwrap();
                    }
                    release.send(()).unwrap();
                    let mut failure = task.await.unwrap().unwrap_err();
                    assert_eq!(
                        failure.code,
                        ErrorCode::Conflict,
                        "{mode}/{boundary}/{action}/{late_error}"
                    );
                    assert!(failure.take_caller_credential_update().is_none());
                    assert!(provider.take_response_credential().unwrap().is_none());
                    if mode != "caller" && action != "logout" {
                        assert_eq!(read(&store, alias), replacement);
                    }
                    assert_eq!(read(&store, "other").token(), "unrelated-pacm");
                    wire.await.unwrap();
                }
            }
        }
    }
}

#[tokio::test]
async fn purchases_cancellation_and_timeout_at_every_boundary_retain_only_verified_rotations() {
    for cancel in [false, true] {
        for boundary in 0..7 {
            let mut responses = frames(&["123", "124", "125"], &["124", "125"]);
            responses.truncate(boundary + 1);
            let (mut provider, seen, _release, wire) = gated(responses).await;
            provider.client = provider
                .client
                .with_session_test_timeout(Duration::from_millis(250));
            let (store, original, alias) = setup(&mut provider, "caller");
            let provider = Arc::new(provider);
            let operation = provider.clone();
            let task =
                tokio::spawn(
                    async move { operation.account_purchased_tracks(&request(alias)).await },
                );
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert!(provider.take_response_credential().unwrap().is_none());
            } else {
                let mut failure = task.await.unwrap().unwrap_err();
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
                            "verified-1"
                        } else {
                            "verified-2"
                        }
                    );
                }
            }
            assert_eq!(read(&store, alias), original);
            wire.abort();
            assert!(wire.await.unwrap_err().is_cancelled());
        }
    }
}

#[tokio::test]
async fn purchases_reverse_account_completion_keeps_each_uid_and_rotation_isolated() {
    let (mut first, seen, release, first_wire) =
        gated(frames(&["123", "124", "125"], &["124", "125"])).await;
    let (store, _, alias) = setup(&mut first, "named");
    let second_frames = frames(&["999", "124", "125"], &["124", "125"])
        .into_iter()
        .map(|s| {
            s.replace("\"userId\":\"111\"", "\"userId\":\"222\"")
                .replace("verified-", "secondok-")
                .replace("candidate-", "secondcan-")
        })
        .collect();
    let (mut second, second_wire) = server(second_frames).await;
    second.credential_store = Some(store.clone());
    let task = tokio::spawn(async move { first.account_purchased_tracks(&request(alias)).await });
    seen.await.unwrap();
    let second_page = second
        .account_purchased_tracks(&request("other"))
        .await
        .unwrap();
    release.send(()).unwrap();
    let first_page = task.await.unwrap().unwrap();
    assert_eq!(first_page.pagination.extensions["source_user_id"], "111");
    assert_eq!(second_page.pagination.extensions["source_user_id"], "222");
    assert_ne!(
        first_page.pagination.extensions["source_snapshot_id"],
        second_page.pagination.extensions["source_snapshot_id"]
    );
    assert_eq!(read(&store, alias).token(), "verified-3");
    assert_eq!(read(&store, "other").token(), "secondok-3");
    first_wire.await.unwrap();
    second_wire.await.unwrap();
}

#[tokio::test]
async fn purchases_invalid_pages_missing_accounts_and_album_calls_do_not_use_public_fallback() {
    let provider = MiguProvider::from_client(MiguClient::test_client());
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert_eq!(
            provider
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
        provider
            .account_purchased_tracks(&request("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        provider
            .account_purchased_albums(&request("default"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}
