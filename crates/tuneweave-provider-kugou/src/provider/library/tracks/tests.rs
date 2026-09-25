use super::super::tests::{page as library_page, request, row as playlist_row, store_account};
use super::*;
use crate::provider::session::tests::{
    credential, exchange, paused, profile, raw, read, reply, server,
};

const REF: &str = "cloudlist:111:1:7";
fn row(id: u64, sort: u64) -> Value {
    json!({"fileid":id,"sort":sort,"mixsongid":900 + id % 5,"name":format!("Song {id}"),
        "collecttime":id,"timelen":123000,"hash":"1234567890abcdef1234567890abcdef"})
}
fn page(rows: Vec<Value>, total: u64) -> String {
    reply(json!({"userid":111,"listid":7,"type":1,"list_ver":3,"count":total,"info":rows}))
}
fn library() -> String {
    library_page(vec![playlist_row(7, 1)])
}

#[tokio::test]
async fn native_tracks_read_four_pages_and_sort_globally_without_deduplicating_catalogue_songs() {
    let all = (1..=1060).map(|id| row(id, 1060 - id)).collect::<Vec<_>>();
    let mut frames = vec![
        exchange("111", "next").into(),
        profile("111").into(),
        library().into(),
    ];
    frames.extend(all.chunks(300).map(|c| page(c.to_vec(), 1060).into()));
    frames.push(library().into());
    let mut f = server(frames).await;
    let store = store_account(&mut f.provider);
    let other = read(&store, "B");
    let p = f
        .provider
        .playlist_tracks(REF, &request("A", 10, 295))
        .await
        .unwrap();
    assert_eq!(p.pagination.total, Some(1060));
    assert_eq!(p.pagination.next_offset, Some(305));
    assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 4);
    assert_eq!(p.pagination.extensions["ordering"], "sort_ascending");
    assert_eq!(
        p.items
            .iter()
            .map(|t| t.extensions["file_id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        (756..=765).rev().collect::<Vec<_>>()
    );
    assert_eq!(p.items[0].extensions["playlist_position"], 295);
    assert_eq!(p.items[0].extensions["upstream_position"], 764);
    assert_eq!(p.items[0].id, p.items[5].id);
    assert_eq!(p.pagination.extensions["count"], 2); // v8 count is not v3 occurrence count.
    assert_eq!(p.pagination.extensions["track_list_count"], 1060);
    assert_eq!(read(&store, "B"), other);
    assert_eq!(read(&store, "A").native().session.token, "next");
    assert!(f.provider.take_response_credential().unwrap().is_none());
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 8);
    for (i, r) in requests[3..7].iter().enumerate() {
        assert!(r.starts_with("POST /v4/get_list_all_file_v3?"));
        let b: Value = serde_json::from_str(r.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(b["userid"], 111);
        assert_eq!(b["listid"], 7);
        assert_eq!(b["type"], 1);
        assert_eq!(b["token"], "next");
        assert_eq!(b["page"], i + 1);
        assert_eq!(b["pagesize"], 300);
    }
}

#[tokio::test]
async fn metadata_and_tracks_share_content_revisions_independent_of_token_or_alias() {
    let mut revisions = Vec::new();
    for case in ["metadata", "tracks", "changed"] {
        let mut rows = vec![row(81, 0), row(95, 1)];
        if case == "changed" {
            rows[1]["name"] = json!("New title");
        }
        let mut f = server(vec![
            exchange("111", case).into(),
            profile("111").into(),
            library().into(),
            page(rows, 2).into(),
            library().into(),
        ])
        .await;
        let store = store_account(&mut f.provider);
        let caller = f
            .provider
            .caller_scope(&read(&store, "A").caller().unwrap())
            .unwrap();
        let revision = if case == "metadata" {
            let p = caller.playlist(REF, None).await.unwrap();
            assert_eq!(p.track_count, Some(2));
            p.extensions["source_snapshot_id"].clone()
        } else {
            let p = caller
                .playlist_playable_items(REF, &PageRequest::new(1, 1))
                .await
                .unwrap();
            assert_eq!(p.pagination.total, Some(2));
            assert_eq!(p.items.len(), 1);
            p.pagination.extensions["source_snapshot_id"].clone()
        };
        assert!(
            revision
                .as_str()
                .unwrap()
                .starts_with("kugou-cloudlist-v3-")
        );
        revisions.push(revision);
        let update =
            KugouCredential::parse_caller(&caller.take_response_credential().unwrap().unwrap())
                .unwrap();
        assert_eq!(update.native().session.token, case);
        assert_eq!(read(&store, "A").native().session.token, "original");
        f.requests.await.unwrap();
    }
    assert_eq!(revisions[0], revisions[1]);
    assert_ne!(revisions[1], revisions[2]);
}

#[tokio::test]
async fn empty_exact_full_and_out_of_range_native_lists_use_declared_counts_and_preserve_ties() {
    for (count, offset, sorts) in [
        (0, 0, true),
        (300, 0, true),
        (3, 99, true),
        (3, 0, false),
        (3, 0, true),
    ] {
        let rows = (1..=count)
            .map(|id| {
                let mut r = row(id, 0);
                if !sorts {
                    r.as_object_mut().unwrap().remove("sort");
                }
                r
            })
            .collect();
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            library().into(),
            page(rows, count).into(),
            library().into(),
        ])
        .await;
        store_account(&mut f.provider);
        let p = f
            .provider
            .playlist_tracks(REF, &request("A", 100, offset))
            .await
            .unwrap();
        assert_eq!(p.pagination.total, Some(count));
        assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 1);
        assert_eq!(
            p.items.len(),
            if offset > count as u32 {
                0
            } else {
                count.min(100) as usize
            }
        );
        if let Some(t) = p.items.first() {
            assert_eq!(t.extensions["file_id"], 1);
        }
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn native_tracks_reject_changed_counts_versions_duplicates_incomplete_pages_and_budgets() {
    let first = (1..=300).map(|id| row(id, id)).collect::<Vec<_>>();
    for case in [
        "version",
        "count",
        "repeat",
        "short",
        "budget",
        "mixed_sort",
        "unresolved",
    ] {
        let mut frames = vec![
            exchange("111", "next").into(),
            profile("111").into(),
            library().into(),
        ];
        let mut response = json!({"list_ver":3,"count":301,"info":[row(301,301)]});
        let expected = match case {
            "version" | "count" | "repeat" => {
                frames.push(page(first.clone(), 301).into());
                match case {
                    "version" => response["list_ver"] = json!(4),
                    "count" => response["count"] = json!(302),
                    _ => response["info"] = json!([row(1, 1)]),
                }
                ErrorCode::Conflict
            }
            "budget" => {
                response["count"] = json!(38401);
                ErrorCode::UpstreamError
            }
            "mixed_sort" => {
                response["count"] = json!(2);
                let mut r = row(2, 2);
                r.as_object_mut().unwrap().remove("sort");
                response["info"] = json!([row(1, 1), r]);
                ErrorCode::UpstreamError
            }
            "unresolved" => {
                response["count"] = json!(1);
                response["info"][0]["mixsongid"] = json!(0);
                ErrorCode::CapabilityNotSupported
            }
            _ => ErrorCode::UpstreamError,
        };
        frames.push(reply(response).into());
        let length = frames.len();
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        assert_eq!(
            f.provider
                .playlist_tracks(REF, &request("A", 1, 0))
                .await
                .unwrap_err()
                .code,
            expected,
            "{case}"
        );
        assert_eq!(f.requests.await.unwrap().len(), length);
    }
}

#[tokio::test]
async fn metadata_is_bound_to_the_selected_library_before_and_after_the_track_read() {
    for case in [
        "absent",
        "missing_version",
        "changed_version",
        "changed_owner",
        "removed",
    ] {
        let mut before = playlist_row(7, 1);
        if case == "missing_version" {
            before.as_object_mut().unwrap().remove("list_ver");
        }
        let mut frames = vec![
            exchange("111", "next").into(),
            profile("111").into(),
            library_page(if case == "absent" {
                vec![]
            } else {
                vec![before]
            })
            .into(),
        ];
        let expected = match case {
            "absent" => ErrorCode::ResourceNotFound,
            "missing_version" => ErrorCode::UpstreamError,
            _ => {
                frames.push(page(vec![row(1, 0)], 1).into());
                let mut after = playlist_row(7, 1);
                if case == "changed_version" {
                    after["list_ver"] = json!(4);
                }
                if case == "changed_owner" {
                    after["list_create_userid"] = json!(222);
                }
                frames.push(
                    library_page(if case == "removed" {
                        vec![]
                    } else {
                        vec![after]
                    })
                    .into(),
                );
                ErrorCode::Conflict
            }
        };
        let length = frames.len();
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        assert_eq!(
            f.provider.playlist(REF, Some("A")).await.unwrap_err().code,
            expected,
            "{case}"
        );
        assert_eq!(f.requests.await.unwrap().len(), length);
    }
}

#[tokio::test]
async fn native_track_errors_return_only_accepted_rotation_and_keep_other_accounts_untouched() {
    for caller in [false, true] {
        for (response, expected) in [
            (
                raw(json!({"error_code":20010,"data":"do-not-export-v3-payload"})),
                ErrorCode::UpstreamError,
            ),
            (
                raw(json!({"error_code":20017,"data":"do-not-export-v3-payload"})),
                ErrorCode::AuthenticationRequired,
            ),
            (
                reply(json!({"userid":222,"list_ver":3,"count":0,"info":[]})),
                ErrorCode::Conflict,
            ),
        ] {
            let mut f = server(vec![
                exchange("111", "next").into(),
                profile("111").into(),
                library().into(),
                response.into(),
            ])
            .await;
            let store = store_account(&mut f.provider);
            let other = read(&store, "B");
            let source = read(&store, "A");
            let p = if caller {
                f.provider.caller_scope(&source.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut e = p
                .playlist_tracks(REF, &request(if caller { "default" } else { "A" }, 10, 0))
                .await
                .unwrap_err();
            assert_eq!(e.code, expected);
            assert!(!format!("{e:?}").contains("do-not-export-v3-payload"));
            let update = e.take_caller_credential_update();
            assert_eq!(
                update.is_some(),
                caller && expected == ErrorCode::UpstreamError
            );
            if let Some(update) = update {
                assert_eq!(
                    KugouCredential::parse_caller(&update)
                        .unwrap()
                        .native()
                        .session
                        .token,
                    "next"
                );
            }
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), source);
            }
            f.requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn native_tracks_reject_logout_relogin_and_rotation_during_tracks_or_final_metadata() {
    for final_metadata in [false, true] {
        for action in ["logout", "relogin", "rotate", "expired_relogin"] {
            let (last, resume) = paused(if action == "expired_relogin" {
                raw(json!({"status":0,"error_code":20017,"data":null}))
            } else if final_metadata {
                library()
            } else {
                page(vec![row(1, 0)], 1)
            });
            let mut frames = vec![
                exchange("111", "next").into(),
                profile("111").into(),
                library().into(),
            ];
            if final_metadata {
                frames.push(page(vec![row(1, 0)], 1).into());
            }
            frames.push(last);
            let length = frames.len();
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let provider = f.provider.clone();
            let task =
                tokio::spawn(
                    async move { provider.playlist_tracks(REF, &request("A", 1, 0)).await },
                );
            for _ in 0..length {
                f.seen.recv().await.unwrap();
            }
            let replacement = if action == "rotate" {
                let old = read(&store, "A");
                let mut next = old.native().session.clone();
                next.token = "newer".into();
                old.rotate(next).unwrap()
            } else {
                credential("111", "relogin")
            };
            if action == "logout" {
                store.remove(Platform::Kugou, "A").unwrap();
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut e = task.await.unwrap().unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict);
            assert!(e.take_caller_credential_update().is_none());
            if action != "logout" {
                assert_eq!(read(&store, "A"), replacement);
            }
            f.requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn caller_replacement_discards_native_tracks_and_late_rotation_without_modifying_server() {
    for final_metadata in [false, true] {
        let (last, resume) = paused(if final_metadata {
            library()
        } else {
            page(vec![row(1, 0)], 1)
        });
        let mut frames = vec![
            exchange("111", "next").into(),
            profile("111").into(),
            library().into(),
        ];
        if final_metadata {
            frames.push(page(vec![row(1, 0)], 1).into());
        }
        frames.push(last);
        let length = frames.len();
        let mut f = server(frames).await;
        let store = store_account(&mut f.provider);
        let saved = read(&store, "A");
        let caller = f.provider.caller_scope(&saved.caller().unwrap()).unwrap();
        let scoped = caller.clone();
        let task =
            tokio::spawn(
                async move { scoped.playlist_tracks(REF, &PageRequest::new(10, 0)).await },
            );
        for _ in 0..length {
            f.seen.recv().await.unwrap();
        }
        let replacement = credential("222", "replacement");
        *caller.caller_credential.as_ref().unwrap().lock().unwrap() = Some(replacement.clone());
        resume.send(()).unwrap();
        let mut e = task.await.unwrap().unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert!(e.take_caller_credential_update().is_none());
        assert!(caller.take_response_credential().unwrap().is_none());
        assert_eq!(
            *caller.caller_credential.as_ref().unwrap().lock().unwrap(),
            Some(replacement)
        );
        assert_eq!(read(&store, "A"), saved);
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn native_tracks_validate_window_web_scope_and_cross_user_before_network() {
    let mut f = server(vec![]).await;
    let store = store_account(&mut f.provider);
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert_eq!(
            f.provider
                .playlist_tracks(REF, &request("A", limit, offset))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .playlist_tracks("cloudlist:222:1:7", &request("A", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let web =
        KugouCredential::verified_web(crate::web::WebSession::test_session("111", "web-cookie"))
            .unwrap();
    store.put(&web.stored("W").unwrap()).unwrap();
    assert_eq!(
        f.provider
            .playlist_tracks(REF, &request("W", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(read(&store, "W"), web);
    assert!(f.requests.await.unwrap().is_empty());
}
