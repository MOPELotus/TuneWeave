use super::*;
use crate::client::account_search::tests::page;
use std::time::Duration;
fn query(owner: &str, kind: SearchKind) -> SearchQuery {
    let mut q = SearchQuery::tracks(" q ", 5, 18);
    q.kind = kind;
    q.account = (owner != "caller").then(|| owner.into());
    q
}
fn replies(kind: SearchKind) -> Vec<String> {
    vec![
        account_reply("123456", Some("sessionid_ss=verified-session; Path=/")),
        crate::test_http::json(
            &page(kind, 0, 45).to_string(),
            Some("sessionid_ss=first-page-session; Path=/"),
        ),
        crate::test_http::json(
            &page(kind, 20, 45).to_string(),
            Some("sessionid_ss=last-page-session; Path=/"),
        ),
    ]
}
fn provider(f: &SessionFixture, owner: &str, source: &SodaCredential) -> SodaProvider {
    if owner == "caller" {
        f.provider
            .caller_credential_scope(&caller_from(source))
            .unwrap()
    } else {
        f.provider.clone()
    }
}
#[tokio::test]
async fn account_search_all_types_and_three_owners_preserve_pagination_rotations_and_public_metadata()
 {
    for owner in ["default", "personal", "caller"] {
        for kind in [
            SearchKind::Track,
            SearchKind::Album,
            SearchKind::Artist,
            SearchKind::Playlist,
        ] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            for alias in ["default", "personal", "other"] {
                f.put(alias, &source);
            }
            let (origin, server) = crate::test_http::serve(replies(kind)).await;
            f.provider.client = f
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin.clone());
            let p = provider(&f, owner, &source);
            let result = p.search_catalog(&query(owner, kind)).await.unwrap();
            assert_eq!(result.items.len(), 5);
            assert_eq!(result.pagination.next_offset, Some(23));
            assert!(result.pagination.has_more);
            assert_eq!(result.pagination.extensions["source_user_id"], "123456");
            assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 2);
            let value = serde_json::to_value(&result).unwrap();
            let serialized = value.to_string();
            for secret in [
                "session-secret",
                "verified-session",
                "first-page-session",
                "last-page-session",
                "discard-private-player",
            ] {
                assert!(!serialized.contains(secret));
            }
            assert!(serialized.contains("1018") && serialized.contains("1022"));
            for alias in ["default", "personal", "other"]
                .into_iter()
                .filter(|a| *a != owner)
            {
                assert_eq!(
                    f.stored(alias).unwrap().secret(),
                    source.serialize().unwrap()
                );
            }
            if owner == "caller" {
                assert!(
                    p.take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("last-page-session")
                );
            } else {
                assert!(
                    f.stored(owner)
                        .unwrap()
                        .secret()
                        .contains("last-page-session")
                );
                assert!(p.take_response_credential().unwrap().is_none());
            }
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 3);
            assert!(requests[1].contains("sessionid_ss=verified-session"));
            assert!(requests[2].contains("sessionid_ss=first-page-session"));
            let mut ids = Vec::new();
            for (wire, cursor) in requests[1..].iter().zip(["0", "20"]) {
                let u = origin
                    .join(
                        wire.lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap(),
                    )
                    .unwrap();
                let params = u.query_pairs().collect::<BTreeMap<_, _>>();
                assert_eq!(params["cursor"], cursor);
                ids.push(params["search_id"].to_string());
            }
            assert_eq!(ids[0], ids[1]);
        }
    }
}
#[tokio::test]
async fn account_search_every_boundary_observes_logout_same_cookie_relogin_and_account_switch() {
    for owner in ["default", "personal", "caller"] {
        for boundary in 0..3 {
            for action in ["logout", "relogin", "switch"] {
                if owner == "caller" && action == "logout" {
                    continue;
                }
                for failure in [false, true] {
                    let mut f = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    f.put("default", &source);
                    f.put("personal", &source);
                    let mut frames = replies(SearchKind::Track);
                    frames.truncate(boundary + 1);
                    if failure {
                        frames[boundary] =
                            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into();
                    }
                    let paused = crate::test_http::serve_paused_at(frames, boundary).await;
                    f.provider.client = f
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin.clone());
                    let p = provider(&f, owner, &source);
                    let worker = p.clone();
                    let task = tokio::spawn(async move {
                        worker.search(&query(owner, SearchKind::Track)).await
                    });
                    tokio::time::timeout(Duration::from_secs(5), paused.arrived)
                        .await
                        .unwrap()
                        .unwrap();
                    let token = match boundary {
                        0 => "session-secret",
                        1 => "verified-session",
                        _ => "first-page-session",
                    };
                    let next = SodaCredential::test_credential(token)
                        .bind_user(if action == "switch" {
                            "654321"
                        } else {
                            "123456"
                        })
                        .unwrap();
                    if owner == "caller" {
                        *p.caller_credential.as_ref().unwrap().lock().unwrap() = next;
                    } else if action == "logout" {
                        f.store.remove(Platform::Soda, owner).unwrap();
                    } else {
                        f.put(owner, &next);
                    }
                    paused.release.send(()).unwrap();
                    assert_eq!(
                        task.await.unwrap().unwrap_err().code,
                        ErrorCode::Conflict,
                        "{owner}/{boundary}/{action}/{failure}"
                    );
                    assert!(p.take_response_credential().unwrap().is_none());
                    paused.requests.await.unwrap();
                }
            }
        }
    }
}
#[tokio::test]
async fn account_search_cancel_deadline_and_late_bad_page_never_export_partial_results_or_pending_updates()
 {
    for owner in ["personal", "caller"] {
        for boundary in 0..3 {
            for action in ["cancel", "timeout", "invalid"] {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                f.put("personal", &source);
                let mut frames = replies(SearchKind::Album);
                frames.truncate(boundary + 1);
                if action == "invalid" {
                    frames[boundary] =
                        crate::test_http::json("{}", Some("sessionid_ss=unaccepted; Path=/"));
                }
                let paused = crate::test_http::serve_bytes_paused_at_with_hold_timeout(
                    frames.into_iter().map(String::into_bytes).collect(),
                    boundary,
                    Duration::from_secs(60),
                )
                .await;
                f.provider.client = f
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin.clone());
                let p = provider(&f, owner, &source);
                let worker = p.clone();
                let task = tokio::spawn(async move {
                    worker
                        .read_account_search(
                            &query(owner, SearchKind::Album),
                            // The total deadline precedes the20s HTTP timeout,
                            // but leaves real network setup outside the timing assertion.
                            Duration::from_secs(if action == "timeout" { 15 } else { 45 }),
                        )
                        .await
                });
                tokio::time::timeout(Duration::from_secs(10), paused.arrived)
                    .await
                    .unwrap()
                    .unwrap();
                if action == "cancel" {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else if action == "timeout" {
                    // Pause only after the selected TCP request reached its gate.
                    // With all remaining work waiting, Tokio advances to the actual
                    // operation deadline, before the HTTP and mock-server deadlines.
                    tokio::time::pause();
                    let error = task.await.unwrap().unwrap_err();
                    tokio::time::resume();
                    assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                    assert!(error.message.contains("total time budget"));
                } else {
                    paused.release.send(()).unwrap();
                    assert!(task.await.unwrap().is_err());
                }
                assert!(p.take_response_credential().unwrap().is_none());
                if action == "invalid" {
                    paused.requests.await.unwrap();
                } else {
                    paused.requests.abort();
                }
            }
        }
    }
}
#[tokio::test]
async fn account_search_later_cookie_reflection_rejects_even_skipped_earlier_items() {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    f.put("personal", &source);
    let mut frames = replies(SearchKind::Album);
    let mut first = page(SearchKind::Album, 0, 45);
    first["result_groups"][0]["data"][0]["entity"]["album"]["name"] = json!("last-page-session");
    frames[1] = crate::test_http::json(
        &first.to_string(),
        Some("sessionid_ss=first-page-session; Path=/"),
    );
    let (origin, server) = crate::test_http::serve(frames).await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    let p = provider(&f, "caller", &source);
    assert_eq!(
        p.search_catalog(&query("caller", SearchKind::Album))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert!(p.take_response_credential().unwrap().is_none());
    server.await.unwrap();
}
#[tokio::test]
async fn account_search_two_accounts_finishing_in_reverse_order_do_not_share_sources_or_rotations()
{
    let mut a = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    a.put("personal", &source);
    let other = SodaCredential::test_credential("other-session")
        .bind_user("654321")
        .unwrap();
    a.put("other", &other);
    let paused = crate::test_http::serve_paused_at(replies(SearchKind::Album), 2).await;
    a.provider.client = a
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin.clone());
    let worker = a.provider.clone();
    let task = tokio::spawn(async move {
        worker
            .search_catalog(&query("personal", SearchKind::Album))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), paused.arrived)
        .await
        .unwrap()
        .unwrap();
    let mut b = a.provider.clone();
    let frames = replies(SearchKind::Album)
        .into_iter()
        .map(|s| {
            s.replace("123456", "654321")
                .replace("sessionid_ss=", "sessionid_ss=other-")
        })
        .collect();
    let (origin, server) = crate::test_http::serve(frames).await;
    b.client = b.client.with_auth_test_origin(origin);
    let result = b
        .search_catalog(&query("other", SearchKind::Album))
        .await
        .unwrap();
    assert_eq!(result.pagination.extensions["source_user_id"], "654321");
    paused.release.send(()).unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.pagination.extensions["source_user_id"], "123456");
    assert!(
        a.stored("personal")
            .unwrap()
            .secret()
            .contains("last-page-session")
    );
    assert!(
        a.stored("other")
            .unwrap()
            .secret()
            .contains("other-last-page-session")
    );
    server.await.unwrap();
    paused.requests.await.unwrap();
}
#[tokio::test]
async fn account_search_real_resolver_uses_selected_pc_candidates_then_independent_media_authorization()
 {
    use tuneweave_core::{
        ArtistSummary, ProviderRegistry, ResolveRequest, ResourceRef, StreamResolver,
    };
    for caller in [false, true] {
        for matches in [false, true] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            f.put("personal", &source);
            let media: serde_json::Value =
                serde_json::from_slice(&crate::client::test_account_track_fixture(false)).unwrap();
            let mut search = page(SearchKind::Track, 0, 1);
            search["result_groups"][0]["data"][0]["entity"]["track"] = media["track"].clone();
            if !matches {
                search["result_groups"][0]["data"][0]["entity"]["track"]["name"] =
                    json!("Completely different work");
            }
            let mut frames = vec![
                account_reply("123456", Some("sessionid_ss=verified-session; Path=/")),
                crate::test_http::json(
                    &search.to_string(),
                    Some("sessionid_ss=search-session; Path=/"),
                ),
            ];
            if matches {
                frames.extend([
                    account_reply(
                        "123456",
                        Some("sessionid_ss=media-verified-session; Path=/"),
                    ),
                    crate::test_http::json(
                        &media.to_string(),
                        Some("sessionid_ss=final-media-session; Path=/"),
                    ),
                ]);
            }
            let (base, server) = crate::test_http::serve(frames).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(base);
            let p = provider(&f, if caller { "caller" } else { "personal" }, &source);
            let mut registry = ProviderRegistry::new();
            registry.register(p.clone()).unwrap();
            let resolver = StreamResolver::new(registry, vec![]);
            let mut origin = Track::new(
                ResourceRef::new(Platform::Netease, "17").unwrap(),
                media["track"]["name"].as_str().unwrap(),
            );
            origin.duration_ms = media["track"]["duration"].as_u64();
            origin.artists = vec![ArtistSummary {
                name: media["track"]["artists"][0]["name"]
                    .as_str()
                    .unwrap()
                    .into(),
                resource_ref: None,
            }];
            let result = resolver
                .resolve(
                    &origin,
                    &ResolveRequest {
                        playback_platforms: vec![Platform::Soda],
                        fallback: false,
                        accounts: [(
                            Platform::Soda,
                            if caller {
                                "default".into()
                            } else {
                                "personal".into()
                            },
                        )]
                        .into(),
                        ..Default::default()
                    },
                )
                .await;
            if matches {
                assert_eq!(result.unwrap().resolved_platform, Platform::Soda);
            } else {
                assert_eq!(result.unwrap_err().code, ErrorCode::MatchRejected);
            }
            let wires = server.await.unwrap();
            assert_eq!(wires.len(), if matches { 4 } else { 2 });
            assert!(wires[1].starts_with("GET /luna/pc/search/track?"));
            assert!(wires[1].contains("sessionid_ss=verified-session"));
            if matches {
                assert!(wires[2].contains("sessionid_ss=search-session"));
                assert!(wires[3].contains("sessionid_ss=media-verified-session"));
            }
        }
    }
}
#[tokio::test]
async fn account_search_empty_out_of_range_and_invalid_preflight_are_explicit() {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    f.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", None),
        crate::test_http::json(&page(SearchKind::Album, 0, 30).to_string(), None),
        crate::test_http::json(&page(SearchKind::Album, 20, 30).to_string(), None),
    ])
    .await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    let mut q = query("personal", SearchKind::Album);
    q.offset = 40;
    let page = f.provider.search_catalog(&q).await.unwrap();
    assert!(page.items.is_empty());
    assert!(!page.pagination.has_more);
    server.await.unwrap();
    let f = SessionFixture::new();
    for case in 0..6 {
        let mut q = query("missing", SearchKind::Track);
        match case {
            0 => q.query = "bad\nquery".into(),
            1 => q.limit = 0,
            2 => q.offset = u32::MAX,
            3 => q.variant = SearchVariant::Cloud,
            4 => q.offset = 2560,
            _ => q.search_id = Some("foreign".into()),
        }
        assert_eq!(
            f.provider.search(&q).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .search(&query("missing", SearchKind::Track))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn account_search_sparse_and_empty_continuation_pages_count_visible_items_not_cursor_positions()
 {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    f.put("personal", &source);
    let mut first = page(SearchKind::Album, 0, 60);
    first["result_groups"][0]["data"]
        .as_array_mut()
        .unwrap()
        .truncate(10);
    let mut empty = page(SearchKind::Album, 20, 60);
    empty["result_groups"][0]["data"] = json!([]);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", None),
        crate::test_http::json(&first.to_string(), None),
        crate::test_http::json(&empty.to_string(), None),
        crate::test_http::json(&page(SearchKind::Album, 40, 60).to_string(), None),
    ])
    .await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    let mut q = query("personal", SearchKind::Album);
    q.offset = 8;
    let result = f.provider.search_catalog(&q).await.unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|v| match v {
                SearchItem::Album(a) => a.id.as_str(),
                _ => panic!("kind"),
            })
            .collect::<Vec<_>>(),
        ["1008", "1009", "1040", "1041", "1042"]
    );
    assert_eq!(result.pagination.next_offset, Some(13));
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 3);
    let requests = server.await.unwrap();
    for (wire, cursor) in requests[1..].iter().zip(["0", "20", "40"]) {
        assert!(wire.contains(&format!("cursor={cursor}&")));
    }
}

#[tokio::test]
async fn account_search_empty_pages_with_advancing_cursors_stop_at_the_total_page_budget() {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    f.put("personal", &source);
    let mut responses = vec![account_reply("123456", None)];
    for i in 0..128 {
        responses.push(crate::test_http::json(&json!({"status_info":{"now":1,"now_ts_ms":1000},"result_groups":[{"id":"albums","has_more":true,"next_cursor":((i+1)*20).to_string(),"data":[]}]}).to_string(),None));
    }
    let (origin, server) = crate::test_http::serve(responses).await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    assert_eq!(
        f.provider
            .search_catalog(&query("personal", SearchKind::Album))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(server.await.unwrap().len(), 129);
}
