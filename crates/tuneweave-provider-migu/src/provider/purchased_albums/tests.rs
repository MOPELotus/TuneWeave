use super::*;
use crate::credential::MiguCredential;
use crate::provider::account_media::tests::{reply, server, setup};
use crate::provider::session::tests::{gated, profile, read, stored};

fn row(id: &str, kind: &str) -> serde_json::Value {
    json!({"contentId":id,"resourceType":kind,"title":"Album","singer":"Artist","totalCount":"10","copyrightId":"ABC123"})
}
fn pages() -> Vec<serde_json::Value> {
    vec![
        json!({"resources":[row("77","2003"),row("77","5")],"hasNextPage":true}),
        json!({"resources":[row("77","2003"),row("88","2003")],"hasNextPage":false}),
    ]
}
fn frames(pages: &[serde_json::Value]) -> Vec<String> {
    let mut responses = vec![profile("111", "pacmtoken: verified-0\r\n")];
    for (i, page) in pages.iter().chain(pages.iter()).enumerate() {
        responses.push(reply(
            json!({"code":"000000","data":page}),
            Some(&format!("candidate-{}", i + 1)),
        ));
        responses.push(profile(
            "111",
            &format!("pacmtoken: verified-{}\r\n", i + 1),
        ));
    }
    responses
}
fn request(alias: &str) -> PageRequest {
    PageRequest {
        limit: 3,
        offset: 1,
        account: Some(alias.into()),
    }
}

#[tokio::test]
async fn purchased_albums_full_scans_use_selected_pacm_and_preserve_mixed_order_and_duplicates() {
    for mode in ["default", "named", "caller"] {
        let (mut provider, wire) = server(frames(&pages())).await;
        let (store, original, alias) = setup(&mut provider, mode);
        let result = provider
            .account_purchased_albums(&request(alias))
            .await
            .unwrap();
        assert_eq!(result.pagination.total, Some(4));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 4);
        assert_eq!(result.pagination.extensions["source_user_id"], "111");
        assert!(result.items[0].album.is_none());
        assert_eq!(result.items[0].digital_album.as_ref().unwrap().id, "77");
        assert_eq!(result.items[1].album.as_ref().unwrap().id, "77");
        assert!(result.items[1].digital_album.is_none());
        assert_eq!(result.items[2].album.as_ref().unwrap().id, "88");
        for (i, item) in result.items.iter().enumerate() {
            assert_eq!(item.extensions["source_position"], i + 1);
        }
        let json = serde_json::to_string(&result).unwrap();
        for secret in ["initial-pacm", "candidate-", "verified-", "do-not-retain"] {
            assert!(!json.contains(secret));
        }
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            assert_eq!(
                MiguCredential::parse_caller(
                    &provider.take_response_credential().unwrap().unwrap()
                )
                .unwrap()
                .token(),
                "verified-4"
            );
        } else {
            assert_eq!(read(&store, alias).token(), "verified-4");
            assert!(provider.take_response_credential().unwrap().is_none());
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        let wire = wire.await.unwrap();
        assert_eq!(wire.len(), 9);
        for (position, index) in [1, 3, 5, 7].into_iter().enumerate() {
            assert!(wire[index].starts_with(&format!(
                "GET /strategy/album-subscription/list/v1.0?pageNumber={}&pageSize=50 ",
                position % 2 + 1
            )));
            assert!(wire[index].contains(&format!("pacmtoken: verified-{position}\r\n")));
            assert!(!wire[index].to_ascii_lowercase().contains("\r\ncookie:"));
            assert!(wire[index].contains("channel: 014X031\r\n"));
            assert!(wire[index].contains("version: 6.8.8\r\n"));
            assert!(
                wire[index + 1].contains(&format!("pacmtoken: candidate-{}\r\n", position + 1))
            );
        }
    }
}

#[tokio::test]
async fn purchased_albums_empty_outside_windows_and_unresolved_records_remain_distinct() {
    for kind in ["empty", "outside", "unresolved"] {
        let mut values = pages();
        if kind == "empty" {
            values = vec![json!({"resources":[],"hasNextPage":false})];
        }
        if kind == "unresolved" {
            values[0]["resources"][1]
                .as_object_mut()
                .unwrap()
                .remove("title");
        }
        let (mut provider, wire) = server(frames(&values)).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let mut request = request(alias);
        if kind == "outside" {
            request.offset = 100;
        }
        let result = provider.account_purchased_albums(&request).await.unwrap();
        if kind == "unresolved" {
            let item = &result.items[0];
            assert!(item.album.is_none() && item.digital_album.is_none());
            assert_eq!(item.extensions["resource_type"], "5");
            assert_eq!(item.extensions["resource_ref"], "migu:77");
            assert_eq!(item.extensions["catalogue_resolved"], false);
        } else {
            assert!(result.items.is_empty());
            assert!(!result.pagination.has_more);
            assert_eq!(
                result.pagination.total,
                Some(if kind == "empty" { 0 } else { 4 })
            );
        }
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn purchased_albums_later_pages_metadata_and_boundaries_must_match_the_first_full_scan() {
    for change in ["id", "type", "title", "count", "cover", "layout"] {
        let values = pages();
        let mut responses = frames(&values);
        let mut later = values[1].clone();
        match change {
            "id" => later["resources"][1]["contentId"] = json!("99"),
            "type" => later["resources"][1]["resourceType"] = json!("5"),
            "title" => later["resources"][1]["title"] = json!("Changed title"),
            "count" => later["resources"][1]["totalCount"] = json!("11"),
            "cover" => {
                later["resources"][1]["imgItems"] = json!([{"img":"https://d.musicapp.migu.cn/prod/file-service/file-down01/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/cccccccccccccccccccccccccccccccc"}])
            }
            _ => {
                let mut first = values[0].clone();
                first["resources"]
                    .as_array_mut()
                    .unwrap()
                    .push(later["resources"].as_array_mut().unwrap().remove(0));
                responses[5] = reply(json!({"code":"000000","data":first}), Some("candidate-3"));
            }
        }
        responses[7] = reply(json!({"code":"000000","data":later}), Some("candidate-4"));
        let (mut provider, wire) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "caller");
        let mut failure = provider
            .account_purchased_albums(&request(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::Conflict, "{change}");
        assert!(failure.take_caller_credential_update().is_none());
        assert!(provider.take_response_credential().unwrap().is_none());
        wire.await.unwrap();
    }
}

#[tokio::test]
async fn purchased_albums_invalid_pages_and_transport_failures_never_become_empty_success() {
    for mode in ["named", "caller"] {
        for kind in [
            "missing-end",
            "unknown-type",
            "empty-more",
            "repeated-page",
            "business",
            "auth",
            "wrong-uid",
            "oversize",
            "html",
        ] {
            let mut responses = frames(&pages());
            let (boundary, expected, verified) = match kind {
                "missing-end" => {
                    responses[3] = reply(
                        json!({"code":"000000","data":{"resources":[]}}),
                        Some("candidate-2"),
                    );
                    (4, ErrorCode::UpstreamError, "verified-2")
                }
                "unknown-type" => {
                    responses[3] = reply(
                        json!({"code":"000000","data":{"resources":[row("88","2037")],"hasNextPage":false}}),
                        Some("candidate-2"),
                    );
                    (4, ErrorCode::UpstreamError, "verified-2")
                }
                "empty-more" => {
                    responses[3] = reply(
                        json!({"code":"000000","data":{"resources":[],"hasNextPage":true}}),
                        Some("candidate-2"),
                    );
                    (4, ErrorCode::UpstreamError, "verified-2")
                }
                "repeated-page" => {
                    responses[3] = reply(
                        json!({"code":"000000","data":pages()[0]}),
                        Some("candidate-2"),
                    );
                    (4, ErrorCode::UpstreamError, "verified-2")
                }
                "business" => {
                    responses[3] = reply(
                        json!({"code":"299999","info":"do-not-export","data":{}}),
                        Some("untrusted"),
                    );
                    (3, ErrorCode::UpstreamError, "verified-1")
                }
                "auth" => {
                    responses[3]="HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (3, ErrorCode::AuthenticationRequired, "")
                }
                "wrong-uid" => {
                    responses[4] = profile("222", "pacmtoken: untrusted\r\n");
                    (4, ErrorCode::AuthenticationRequired, "")
                }
                "oversize" => {
                    responses[3]="HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".into();
                    (3, ErrorCode::UpstreamError, "verified-1")
                }
                _ => {
                    responses[3]="HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (3, ErrorCode::UpstreamError, "verified-1")
                }
            };
            responses.truncate(boundary + 1);
            let (mut provider, wire) = server(responses).await;
            let (store, original, alias) = setup(&mut provider, mode);
            let mut failure = provider
                .account_purchased_albums(&request(alias))
                .await
                .unwrap_err();
            assert_eq!(failure.code, expected, "{mode}/{kind}");
            assert!(!format!("{failure:?}").contains("do-not-export"));
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
                        .any(|c| c.account == alias)
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
async fn purchased_albums_source_changes_are_checked_at_every_page_and_verification_boundary() {
    for mode in ["default", "named", "caller"] {
        for boundary in 0..9 {
            for action in ["logout", "same-token-login", "switch-user"] {
                if mode == "caller" && action == "logout" {
                    continue;
                }
                for late_error in [false, true] {
                    let mut responses = frames(&pages());
                    responses.truncate(boundary + 1);
                    if late_error {
                        responses[boundary]="HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    }
                    let (mut provider, seen, release, wire) = gated(responses).await;
                    let (store, _, alias) = setup(&mut provider, mode);
                    let provider = Arc::new(provider);
                    let operation = provider.clone();
                    let task = tokio::spawn(async move {
                        operation.account_purchased_albums(&request(alias)).await
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
async fn purchased_albums_cancel_and_total_timeout_preserve_only_verified_rotations() {
    for cancel in [false, true] {
        for boundary in 0..9 {
            let mut responses = frames(&pages());
            responses.truncate(boundary + 1);
            let (mut provider, seen, _release, wire) = gated(responses).await;
            let (store, original, alias) = setup(&mut provider, "caller");
            let provider = Arc::new(provider);
            let operation = provider.clone();
            let task = tokio::spawn(async move {
                operation
                    .read_purchased_albums_bounded(
                        &request(alias),
                        MAX_BYTES,
                        Duration::from_millis(300),
                    )
                    .await
            });
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
                        format!("verified-{}", (boundary - 1) / 2)
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
async fn purchased_albums_budget_spans_both_scans_and_page_budget_never_returns_partial_records() {
    // A second, individually small response must obey the remaining cumulative budget.
    let values = vec![json!({"resources":[row("77","5")],"hasNextPage":false})];
    let mut responses = frames(&values);
    let bytes = responses[1].split("\r\n\r\n").nth(1).unwrap().len() as u64;
    responses.truncate(4);
    let (mut provider, wire) = server(responses).await;
    let (store, _, alias) = setup(&mut provider, "named");
    let error = provider
        .read_purchased_albums_bounded(&request(alias), bytes * 2 - 1, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert_eq!(read(&store, alias).token(), "verified-1");
    wire.await.unwrap();
    let values = (1..=MAX_PAGES)
        .map(|i| json!({"resources":[row(&i.to_string(),"5")],"hasNextPage":true}))
        .collect::<Vec<_>>();
    let mut responses = frames(&values);
    responses.truncate(1 + MAX_PAGES as usize * 2);
    let (mut provider, wire) = server(responses).await;
    let (_, _, alias) = setup(&mut provider, "named");
    assert_eq!(
        provider
            .account_purchased_albums(&request(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    wire.await.unwrap();
}

#[tokio::test]
async fn purchased_albums_reverse_accounts_keep_identities_separate_and_snapshot_tracks_all_metadata()
 {
    let (mut first, seen, release, first_wire) = gated(frames(&pages())).await;
    let (store, _, alias) = setup(&mut first, "named");
    let second_frames = frames(&pages())
        .into_iter()
        .map(|s| {
            s.replace("\"userId\":\"111\"", "\"userId\":\"222\"")
                .replace("verified-", "secondok-")
                .replace("candidate-", "secondcan-")
        })
        .collect();
    let (mut second, wire) = server(second_frames).await;
    second.credential_store = Some(store.clone());
    let task = tokio::spawn(async move { first.account_purchased_albums(&request(alias)).await });
    seen.await.unwrap();
    let other = second
        .account_purchased_albums(&request("other"))
        .await
        .unwrap();
    release.send(()).unwrap();
    let first = task.await.unwrap().unwrap();
    assert_eq!(first.pagination.extensions["source_user_id"], "111");
    assert_eq!(other.pagination.extensions["source_user_id"], "222");
    assert_ne!(
        first.pagination.extensions["source_snapshot_id"],
        other.pagination.extensions["source_snapshot_id"]
    );
    assert_eq!(read(&store, alias).token(), "verified-4");
    assert_eq!(read(&store, "other").token(), "secondok-4");
    first_wire.await.unwrap();
    wire.await.unwrap();
}

#[tokio::test]
async fn purchased_albums_reject_invalid_requests_missing_credentials_and_reflected_secrets() {
    let provider = MiguProvider::from_client(MiguClient::test_client());
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert_eq!(
            provider
                .account_purchased_albums(&PageRequest {
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
            .account_purchased_albums(&request("absent"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let mut values = pages();
    values[1]["resources"][1]["title"] = json!("verified-4");
    let (mut provider, wire) = server(frames(&values)).await;
    let (_, _, alias) = setup(&mut provider, "named");
    let failure = provider
        .account_purchased_albums(&request(alias))
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::UpstreamError);
    assert!(!format!("{failure:?}").contains("verified-4"));
    wire.await.unwrap();
}
