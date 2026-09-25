use super::*;
use crate::provider::{
    catalog::tests::server,
    session::tests::{Store, gated, profile, read, stored},
};
use std::time::Duration;

fn reply(value: serde_json::Value, token: &str) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: {token}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn page(section: Section, start: u32, count: u32, total: Option<u64>) -> serde_json::Value {
    let field = if section == Section::Created {
        "list"
    } else {
        "collections"
    };
    let mut value = json!({"code":"000000",field:(start..start+count).map(|id|json!({
        "musicListId":id.to_string(),"title":format!("Playlist {id}"),
        "ownerId":if section==Section::Created {"111"}else{"222"},"musicNum":id,
        "actionUrl":"do-not-export"
    })).collect::<Vec<_>>()});
    if let Some(total) = total {
        value["totalCount"] = json!(total);
    }
    value
}
fn replies() -> Vec<String> {
    vec![
        profile("111", "pacmtoken: p1\r\n"),
        reply(page(Section::Created, 1, 20, None), "created-1"),
        profile("111", "pacmtoken: p2\r\n"),
        reply(page(Section::Created, 21, 1, None), "created-2"),
        profile("111", "pacmtoken: p3\r\n"),
        reply(page(Section::Saved, 1, 1, Some(1)), "saved"),
        profile("111", "pacmtoken: p4\r\n"),
    ]
}
fn setup(p: &mut MiguProvider) -> (Arc<Store>, MiguCredential, MiguCredential) {
    let store = Arc::new(Store::default());
    let a = MiguCredential::verified("111".into(), "initial".into()).unwrap();
    let b = MiguCredential::verified("222".into(), "other".into()).unwrap();
    store.put(&stored("A", &a)).unwrap();
    store.put(&stored("B", &b)).unwrap();
    p.credential_store = Some(store.clone());
    (store, a, b)
}
fn request(account: &str, limit: u32, offset: u32) -> PageRequest {
    PageRequest {
        account: Some(account.into()),
        limit,
        offset,
    }
}
#[tokio::test]
async fn account_library_reads_complete_sections_then_slices_and_verifies_every_rotation() {
    for caller in [false, true] {
        for (offset, limit, expected) in [
            (0, 1, vec!["1"]),
            (19, 3, vec!["20", "21", "1"]),
            (22, 5, vec![]),
        ] {
            let (mut p, requests) = server(replies()).await;
            let (store, a, b) = setup(&mut p);
            let alias = if caller {
                p = p.caller_scope(&a.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let page = p
                .account_playlists(&request(alias, limit, offset))
                .await
                .unwrap();
            assert_eq!(
                page.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(page.pagination.total, Some(22));
            assert_eq!(page.pagination.has_more, offset == 0);
            if offset == 19 {
                assert_eq!(page.items[2].extensions["owner_id"], "222");
                assert_eq!(page.items[2].extensions["library_section"], "saved");
            }
            let update = p.take_response_credential().unwrap();
            if caller {
                assert_eq!(
                    MiguCredential::parse_caller(&update.unwrap())
                        .unwrap()
                        .token(),
                    "p4"
                );
                assert_eq!(read(&store, "A"), a);
            } else {
                assert!(update.is_none());
                assert_eq!(read(&store, "A").token(), "p4");
            }
            assert_eq!(read(&store, "B"), b);
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 7);
            for (r, token) in requests.iter().zip([
                "initial",
                "p1",
                "created-1",
                "p2",
                "created-2",
                "p3",
                "saved",
            ]) {
                assert!(r.contains(&format!("pacmtoken: {token}\r\n")));
                assert!(!r.contains("cookie:"));
                assert_eq!(r.matches("\r\nreferer:").count(), 1);
            }
            assert!(
                requests[1].starts_with(
                    "GET /user/h5/my-music-list/v1.0?pageNo=1&pageSize=20&queryType=0 "
                )
            );
            assert!(requests[3].contains("pageNo=2&pageSize=20&queryType=0"));
            assert!(requests[5].starts_with("GET /user/h5/user/collection/v1.0?pageNo=1&pageSize=20&OPType=03&resourceType=2021&type=1 "));
            assert!(requests[1].contains("channel: 014021I\r\n"));
            assert!(requests[1].contains("referer: https://m.music.migu.cn/\r\n"));
            assert!(requests[1].contains("deviceid:"));
            assert!(
                !serde_json::to_string(&page)
                    .unwrap()
                    .contains("do-not-export")
            );
        }
    }
}

#[tokio::test]
async fn library_full_page_requires_another_page_or_explicit_end_and_enforces_budget() {
    for total in [Some(20), None] {
        let mut responses = vec![profile("111", "")];
        let pages = if total.is_some() { 1 } else { MAX_PAGES };
        for index in 0..pages {
            responses.push(reply(
                page(Section::Created, index * 20 + 1, 20, total),
                "next",
            ));
            responses.push(profile("111", ""));
        }
        let (mut p, requests) = server(responses).await;
        setup(&mut p);
        let result = p.user_created_playlists("111", &request("A", 1, 0)).await;
        if total.is_some() {
            let result = result.unwrap();
            assert_eq!(result.items.len(), 1);
            assert_eq!(result.pagination.total, total);
            assert!(result.pagination.has_more);
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::UpstreamError);
        }
        assert_eq!(requests.await.unwrap().len(), (1 + pages * 2) as usize);
    }
}

#[tokio::test]
async fn library_transport_failures_do_not_accept_tokens_or_anonymous_success() {
    for response in [
        reply(json!({"code":"290001"}), "unverified"),
        reply(json!({"code":"299999"}), "unverified"),
        reply(json!({"list":[]}), "unverified"),
        "HTTP/1.1 302 Found\r\nLocation: https://evil.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 262145\r\nConnection: close\r\n\r\n".into(),
    ] {
        let (mut p, requests) = server(vec![profile("111", "pacmtoken: p1\r\n"), response]).await;
        let (_, a, _) = setup(&mut p);
        let p = p.caller_scope(&a.caller().unwrap()).unwrap();
        let failure = p.account_playlists(&request("default", 1, 0)).await.unwrap_err();
        let update = p.take_response_credential().unwrap();
        if failure.code == ErrorCode::AuthenticationRequired {
            assert!(update.is_none());
        } else {
            assert_eq!(failure.code, ErrorCode::UpstreamError);
            assert_eq!(MiguCredential::parse_caller(&update.unwrap()).unwrap().token(), "p1");
        }
        assert_eq!(requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn own_library_routes_select_only_requested_section_and_reject_invalid_scope_without_network()
{
    for section in [Section::Created, Section::Saved] {
        let (mut p, requests) = server(vec![
            profile("111", ""),
            reply(page(section, 1, 0, Some(0)), "next"),
            profile("111", ""),
        ])
        .await;
        setup(&mut p);
        let result = if section == Section::Created {
            p.user_created_playlists("111", &request("A", 10, 0)).await
        } else {
            p.user_favorite_playlists("111", &request("A", 10, 0)).await
        };
        assert!(result.unwrap().items.is_empty());
        assert_eq!(requests.await.unwrap().len(), 3);
    }
    let (mut p, requests) = server(vec![]).await;
    let (_, a, _) = setup(&mut p);
    for (uid, req, code) in [
        ("222", request("A", 10, 0), ErrorCode::PermissionDenied),
        ("bad id", request("A", 10, 0), ErrorCode::InvalidRequest),
        (
            "111",
            request("missing", 10, 0),
            ErrorCode::AuthenticationRequired,
        ),
        ("111", request("A", 0, 0), ErrorCode::InvalidRequest),
        ("111", request("A", 101, 0), ErrorCode::InvalidRequest),
        ("111", request("A", 2, u32::MAX), ErrorCode::InvalidRequest),
    ] {
        assert_eq!(
            p.user_created_playlists(uid, &req).await.unwrap_err().code,
            code
        );
    }
    let caller = p.caller_scope(&a.caller().unwrap()).unwrap();
    assert_eq!(
        caller
            .account_playlists(&request("A", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn library_rejects_incomplete_changed_and_repeated_pages_without_losing_verified_rotation() {
    let first = page(Section::Created, 1, 20, Some(21));
    for (first, second) in [
        (first.clone(), page(Section::Created, 21, 0, Some(21))),
        (first.clone(), page(Section::Created, 21, 1, Some(22))),
        (first.clone(), page(Section::Created, 21, 1, None)),
        (first.clone(), page(Section::Created, 1, 1, Some(21))),
        (
            first.clone(),
            json!({"code":"000000","list":[],"totalCount":21,"hasNext":true}),
        ),
        (first, json!({"code":"000000","data":{"list":[]}})),
    ] {
        let (mut p, requests) = server(vec![
            profile("111", "pacmtoken: p1\r\n"),
            reply(first, "c1"),
            profile("111", "pacmtoken: p2\r\n"),
            reply(second, "c2"),
            profile("111", "pacmtoken: p3\r\n"),
        ])
        .await;
        let (_, a, _) = setup(&mut p);
        let p = p.caller_scope(&a.caller().unwrap()).unwrap();
        let mut e = p
            .user_created_playlists("111", &request("default", 1, 0))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert_eq!(
            MiguCredential::parse_caller(&e.take_caller_credential_update().unwrap())
                .unwrap()
                .token(),
            "p3"
        );
        assert_eq!(
            MiguCredential::parse_caller(&p.take_response_credential().unwrap().unwrap())
                .unwrap()
                .token(),
            "p3"
        );
        assert_eq!(requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn library_late_response_at_each_boundary_cannot_replace_relogin_or_revive_logout() {
    for remove in [false, true] {
        for stage in 1..=7 {
            let (mut p, seen, release, server) = gated(replies()[..stage].to_vec()).await;
            let (store, _, b) = setup(&mut p);
            let task = tokio::spawn(async move { p.account_playlists(&request("A", 10, 0)).await });
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
            assert_eq!(e.code, ErrorCode::Conflict, "stage {stage}");
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
async fn library_timeouts_and_wrong_uid_never_return_unverified_page_tokens() {
    for stage in 1..=7 {
        let (mut p, seen, release, server) = gated(replies()[..stage].to_vec()).await;
        let (_, a, _) = setup(&mut p);
        p.client = p
            .client
            .with_session_test_timeout(Duration::from_millis(300));
        let p = Arc::new(p.caller_scope(&a.caller().unwrap()).unwrap());
        let worker = p.clone();
        let task =
            tokio::spawn(async move { worker.account_playlists(&request("default", 10, 0)).await });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .unwrap()
            .unwrap();
        let e = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamTimeout);
        let expected = match stage {
            1 => None,
            2 | 3 => Some("p1"),
            4 | 5 => Some("p2"),
            _ => Some("p3"),
        };
        assert_eq!(
            p.take_response_credential()
                .unwrap()
                .map(|c| MiguCredential::parse_caller(&c).unwrap().token().to_owned())
                .as_deref(),
            expected
        );
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        drop(release);
    }
    for root in [true, false] {
        for caller in [false, true] {
            let mut body = page(Section::Created, 1, 0, Some(0));
            if root {
                body["userId"] = json!("222");
            } else {
                body["data"] = json!({"userId":"222"});
            }
            let (mut p, requests) = server(vec![
                profile("111", "pacmtoken: p1\r\n"),
                reply(body, "unverified"),
            ])
            .await;
            let (store, a, b) = setup(&mut p);
            let alias = if caller {
                p = p.caller_scope(&a.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let mut e = p
                .account_playlists(&request(alias, 1, 0))
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::AuthenticationRequired);
            assert!(e.take_caller_credential_update().is_none());
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
            assert_eq!(requests.await.unwrap().len(), 2);
        }
    }
}
