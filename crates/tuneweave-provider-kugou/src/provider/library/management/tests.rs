use super::super::tests::store_account;
use super::*;
use crate::KugouLoginClient;
use crate::provider::session::tests::{
    Frame, Store, credential, exchange, paused, profile, raw, read, reply, server,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

mod concept;
mod concept_batch_delete;
mod concept_create;
mod concept_delete;
mod concept_uncollect;
mod visibility;

const REF: &str = "cloudlist:111:0:37";
const GID: &str = "collection_1_222_88_0";
fn row(id: u64) -> Value {
    json!({"listid":id,"type":0,"name":format!("Name {id}"),"is_def":0,"is_pri":1,
    "intro":"  Preserve intro\n","tags":"甲,乙","sort":42,"count":0,"m_count":0,"list_ver":3,
    "global_collection_id":format!("collection_1_111_{id}_0")})
}
fn collection(id: u64) -> Value {
    let mut p = row(id);
    p["type"] = json!(1);
    p["is_pri"] = json!(0);
    p["list_create_userid"] = json!(222);
    p["list_create_listid"] = json!(88);
    p["list_create_gid"] = json!(GID);
    p
}
fn library(rows: Vec<Value>, version: u64) -> String {
    reply(json!({"userid":111,"total_ver":version,"list_count":rows.len(),"info":rows}))
}
fn ack(id: u64, kind: u8, before: u64, after: u64, count: u64) -> Frame {
    reply(json!({"userid":111,"total_ver":after,"pre_total_ver":before,"list_count":count,"info":{"listid":id,"type":kind}})).into()
}
fn accepted(id: u64, kind: u8, before: u64, after: u64, count: u64) -> Frame {
    reply(json!({"userid":111,"total_ver":after,"pre_total_ver":before,"list_count":count,"info":{"code":1,"listid":id,"type":kind}})).into()
}
fn start() -> Vec<Frame> {
    vec![exchange("111", "next").into(), profile("111").into()]
}
fn body(request: &str) -> Value {
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    if head
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("POST /cloudlist.service/v2/delete_list?"))
    {
        let ciphertext = BASE64.decode(body).unwrap();
        let plaintext = crate::client::decrypt_device_registration_response(
            &ciphertext,
            crate::account::library::management::TEST_RANDOM_SEED,
        )
        .unwrap();
        serde_json::from_slice(&plaintext).unwrap()
    } else {
        serde_json::from_str(body).unwrap()
    }
}
fn create(account: Option<&str>) -> PlaylistCreateRequest {
    PlaylistCreateRequest {
        name: "Created".into(),
        visibility: PlaylistVisibility::Private,
        kind: PlaylistKind::Normal,
        account: account.map(str::to_owned),
    }
}
fn update(account: Option<&str>) -> PlaylistUpdateRequest {
    PlaylistUpdateRequest {
        name: Some("Renamed".into()),
        description: None,
        tags: None,
        variant: PlaylistMetadataUpdateVariant::Default,
        account: account.map(str::to_owned),
    }
}
fn delete(ids: &[u64], account: Option<&str>) -> PlaylistDeleteRequest {
    PlaylistDeleteRequest {
        playlist_refs: ids
            .iter()
            .map(|id| ResourceRef::new(Platform::Kugou, format!("cloudlist:111:0:{id}")).unwrap())
            .collect(),
        account: account.map(str::to_owned),
    }
}
fn store_client(provider: &mut KugouProvider, client: KugouLoginClient) -> Arc<Store> {
    let store = store_account(provider);
    let mut source = read(&store, "A").native().session.clone();
    source.client = client;
    store
        .put(
            &KugouCredential::verified(source)
                .unwrap()
                .stored("A")
                .unwrap(),
        )
        .unwrap();
    store
}

#[tokio::test]
async fn creation_uses_acknowledged_new_identity_and_verifies_standard_visibility_for_both_owners()
{
    let client = KugouLoginClient::Standard;
    for caller in [false, true] {
        let mut created = row(37);
        created["name"] = json!("Created");
        let mut frames = start();
        frames.push(library(vec![row(3)], 9).into());
        frames.push(accepted(37, 0, 9, 10, 2));
        frames.push(library(vec![row(3), created], 10).into());
        let mut f = server(frames).await;
        let store = store_client(&mut f.provider, client);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let result = provider
            .create_playlist(&create(if caller { None } else { Some("A") }))
            .await
            .unwrap();
        assert_eq!(result.playlist_ref.id(), REF);
        assert_eq!(result.playlist.unwrap().name, "Created");
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(provider.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 5);
        assert!(all[3].starts_with("POST /cloudlist.service/v5/add_list?"));
        assert_eq!(body(&all[3])["is_pri"], 1);
        assert_eq!(body(&all[3])["list_create_userid"], 0);
    }
}

#[tokio::test]
async fn creation_does_not_guess_ids_or_accept_wrong_visibility_name_counts_or_extra_changes() {
    for case in [
        "existing_id",
        "missing_id",
        "wrong_name",
        "wrong_privacy",
        "not_empty",
        "extra_change",
        "wrong_ack_version",
    ] {
        let mut created = row(37);
        created["name"] = json!("Created");
        if case == "wrong_name" {
            created["name"] = json!("Other");
        }
        if case == "wrong_privacy" {
            created["is_pri"] = json!(0);
        }
        if case == "not_empty" {
            created["count"] = json!(1);
        }
        let mut frames = start();
        frames.push(library(vec![row(3)], 9).into());
        frames.push(if case == "missing_id" {
            reply(json!({})).into()
        } else {
            accepted(
                if case == "existing_id" { 3 } else { 37 },
                0,
                9,
                if case == "wrong_ack_version" { 11 } else { 10 },
                2,
            )
        });
        if !matches!(case, "existing_id" | "missing_id") {
            let mut other = row(3);
            if case == "extra_change" {
                other["name"] = json!("Changed elsewhere");
            }
            frames.push(library(vec![other, created], 10).into());
        }
        let n = frames.len();
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let e = f
            .provider
            .create_playlist(&create(Some("A")))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), n);
    }
}

#[tokio::test]
async fn renaming_preserves_exact_intro_tags_privacy_sort_and_unrelated_playlists() {
    for caller in [false, true] {
        let mut changed = row(37);
        changed["name"] = json!("Renamed");
        changed["list_ver"] = json!(4);
        let mut frames = start();
        frames.push(library(vec![row(37), row(3)], 9).into());
        frames.push(accepted(37, 0, 9, 10, 2));
        frames.push(library(vec![changed, row(3)], 10).into());
        let mut f = server(frames).await;
        let store = store_account(&mut f.provider);
        let saved = read(&store, "A");
        let provider = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let result = provider
            .update_playlist(REF, &update(if caller { None } else { Some("A") }))
            .await
            .unwrap();
        let p = result.playlist.unwrap();
        assert_eq!(p.description, "  Preserve intro\n");
        assert_eq!(p.tags, ["甲", "乙"]);
        assert_eq!(p.extensions["is_private"], true);
        if caller {
            assert_eq!(read(&store, "A"), saved);
        }
        let all = f.requests.await.unwrap();
        let body = body(&all[3]);
        assert_eq!(body["sort"], 42);
        assert_eq!(body["is_pri"], 1);
        assert_eq!(body["intro"], "  Preserve intro\n");
        assert_eq!(body["tags"], "甲,乙");
        assert!(body.get("token").is_none());
    }
}

#[tokio::test]
async fn metadata_updates_support_explicit_empty_values_and_avoid_observed_noop_writes() {
    for noop in [false, true] {
        let mut request = update(Some("A"));
        request.name = Some(if noop { "Name 37" } else { "Renamed" }.into());
        if !noop {
            request.description = Some(String::new());
            request.tags = Some(vec![]);
        }
        let mut frames = start();
        frames.push(library(vec![row(37)], 9).into());
        if !noop {
            frames.push(accepted(37, 0, 9, 10, 1));
            let mut changed = row(37);
            changed["name"] = json!("Renamed");
            changed["intro"] = json!("");
            changed["tags"] = json!("");
            frames.push(library(vec![changed], 10).into());
        }
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let result = f.provider.update_playlist(REF, &request).await.unwrap();
        assert_eq!(result.extensions["changed"], !noop);
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), if noop { 3 } else { 5 });
        if !noop {
            assert_eq!(body(&all[3])["intro"], "");
            assert_eq!(body(&all[3])["tags"], "");
        }
    }
}

#[tokio::test]
async fn metadata_and_delete_preflight_protect_system_lists_and_unknown_preserved_fields() {
    for case in [
        "system",
        "missing_marker",
        "intro",
        "tags",
        "sort",
        "is_pri",
    ] {
        let mut selected = row(37);
        if case == "system" {
            selected["is_def"] = json!(2);
        } else {
            selected
                .as_object_mut()
                .unwrap()
                .remove(if case == "missing_marker" {
                    "is_def"
                } else {
                    case
                });
        }
        let mut frames = start();
        frames.push(library(vec![selected], 9).into());
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let e = f
            .provider
            .update_playlist(REF, &update(Some("A")))
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none());
        assert_eq!(
            e.code,
            if case == "system" {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::CapabilityNotSupported
            }
        );
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let mut system = row(38);
    system["is_def"] = json!(1);
    let mut frames = start();
    frames.push(library(vec![row(37), system], 9).into());
    let mut f = server(frames).await;
    store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .delete_playlists(&delete(&[37, 38], Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn batch_delete_confirms_each_item_and_reports_partial_failure_without_retrying() {
    for caller in [false, true] {
        for fail in [false, true] {
            let mut frames = start();
            frames.push(library(vec![row(37), row(38), row(39), row(3)], 9).into());
            frames.push(ack(37, 0, 9, 10, 3));
            frames.push(library(vec![row(38), row(39), row(3)], 10).into());
            if fail {
                frames.push(raw(json!({"status":0,"error_code":20010})).into());
            } else {
                frames.push(ack(38, 0, 10, 11, 2));
                frames.push(library(vec![row(39), row(3)], 11).into());
                frames.push(ack(39, 0, 11, 12, 1));
                frames.push(library(vec![row(3)], 12).into());
            }
            let n = frames.len();
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let result = provider
                .delete_playlists(&delete(
                    &[37, 38, 39],
                    if caller { None } else { Some("A") },
                ))
                .await;
            if fail {
                let mut e = result.unwrap_err();
                assert_eq!(e.code, ErrorCode::UpstreamError);
                assert!(!e.retryable);
                assert_eq!(
                    e.details["confirmed_refs"],
                    json!(["kugou:cloudlist:111:0:37"])
                );
                assert_eq!(
                    e.details["unconfirmed_refs"],
                    json!(["kugou:cloudlist:111:0:38"])
                );
                assert_eq!(
                    e.details["not_attempted_refs"],
                    json!(["kugou:cloudlist:111:0:39"])
                );
                assert_eq!(e.take_caller_credential_update().is_some(), caller);
            } else {
                assert_eq!(result.unwrap().playlist_refs.len(), 3);
            }
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            let all = f.requests.await.unwrap();
            assert_eq!(all.len(), n);
            assert_eq!(
                all.iter()
                    .filter(|r| r.starts_with("POST /cloudlist.service/v2/delete_list?"))
                    .count(),
                if fail { 2 } else { 3 }
            );
            assert!(
                all.iter()
                    .all(|r| !r.starts_with("POST /cloudlist.service/v3/delete_list?"))
            );
            for (index, request) in all
                .iter()
                .filter(|r| r.starts_with("POST /cloudlist.service/v2/delete_list?"))
                .enumerate()
            {
                let body = body(request);
                assert_eq!(body["listid"], 37 + index as u64);
                assert_eq!(body["total_ver"], 9 + index as u64);
                assert_eq!(body["type"], 0);
                assert!(body.get("userid").is_none());
                assert!(body.get("token").is_none());
            }
        }
    }
}

#[tokio::test]
async fn delete_and_metadata_readbacks_reject_unrelated_changes_or_unapplied_operations() {
    for rename in [false, true] {
        for unrelated_change in [false, true] {
            let mut selected = row(37);
            if rename && unrelated_change {
                selected["name"] = json!("Renamed");
            }
            let mut other = row(3);
            if unrelated_change {
                other["name"] = json!("Other edit");
            }
            let rows = if rename || !unrelated_change {
                vec![selected, other]
            } else {
                vec![other]
            };
            let mut frames = start();
            frames.push(library(vec![row(37), row(3)], 9).into());
            frames.push(if rename {
                accepted(37, 0, 9, 10, rows.len() as u64)
            } else {
                ack(37, 0, 9, 10, rows.len() as u64)
            });
            frames.push(library(rows, 10).into());
            let mut f = server(frames).await;
            store_account(&mut f.provider);
            let e = if rename {
                f.provider
                    .update_playlist(REF, &update(Some("A")))
                    .await
                    .err()
                    .unwrap()
            } else {
                f.provider
                    .delete_playlists(&delete(&[37], Some("A")))
                    .await
                    .err()
                    .unwrap()
            };
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            assert_eq!(f.requests.await.unwrap().len(), 5);
        }
    }
}

#[tokio::test]
async fn collection_uses_verified_source_gid_and_uncollects_only_the_local_type_one_entry() {
    for caller in [false, true] {
        for add in [false, true] {
            let mut frames = start();
            frames.push(
                library(
                    if add {
                        vec![row(3)]
                    } else {
                        vec![row(3), collection(37)]
                    },
                    9,
                )
                .into(),
            );
            if add {
                frames.push(reply(json!([{"global_collection_id":GID,"parent_global_collection_id":GID,"list_create_gid":GID,
                    "name":"Original collection","list_create_userid":222,"list_create_listid":88,"listid":88,"is_pri":0}])).into());
                frames.push(library(vec![row(3)], 9).into());
            }
            frames.push(if add {
                accepted(37, 1, 9, 10, 2)
            } else {
                ack(37, 1, 9, 10, 1)
            });
            frames.push(
                library(
                    if add {
                        vec![row(3), collection(37)]
                    } else {
                        vec![row(3)]
                    },
                    10,
                )
                .into(),
            );
            let mut f = server(frames).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let result = provider
                .set_playlist_subscription(GID, add, if caller { None } else { Some("A") })
                .await
                .unwrap();
            assert_eq!(result.resource_ref.id(), GID);
            assert_eq!(result.subscribed, add);
            assert_eq!(
                result.extensions["account_playlist_ref"],
                "kugou:cloudlist:111:1:37"
            );
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            let all = f.requests.await.unwrap();
            let write = body(&all[if add { 5 } else { 3 }]);
            assert_eq!(write["type"], 1);
            if add {
                assert_eq!(write["list_create_userid"], 222);
                assert_eq!(write["list_create_listid"], 0);
                assert_eq!(write["list_create_gid"], GID);
                assert!(!all[3].contains("token="));
            } else {
                assert_eq!(write["listid"], 37);
            }
        }
    }
}

#[tokio::test]
async fn collection_noops_and_missing_or_duplicate_source_identities_never_write() {
    for case in ["already", "absent", "qualified", "missing", "duplicate"] {
        let mut rows = vec![row(3)];
        if case != "absent" {
            let mut c = collection(37);
            if case == "missing" {
                c.as_object_mut().unwrap().remove("list_create_gid");
            }
            rows.push(c);
        }
        if case == "duplicate" {
            rows.push(collection(38));
        }
        let mut frames = start();
        frames.push(library(rows, 9).into());
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let result = f
            .provider
            .set_playlist_subscription(
                if case == "qualified" {
                    "cloudlist:111:1:37"
                } else {
                    GID
                },
                case != "absent",
                Some("A"),
            )
            .await;
        if matches!(case, "missing" | "duplicate") {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().extensions["changed"], false);
        }
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn management_relogin_during_write_or_readback_never_exports_or_overwrites_old_credentials() {
    for caller in [false, true] {
        for readback in [false, true] {
            let mut frames = start();
            frames.push(library(vec![row(37)], 9).into());
            let (last, resume) = if readback {
                frames.push(ack(37, 0, 9, 10, 0));
                paused(library(vec![], 10))
            } else {
                paused(reply(json!({"userid":111,"listid":37,"type":0})))
            };
            frames.push(last);
            let n = frames.len();
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let p = provider.clone();
            let task = tokio::spawn(async move {
                p.delete_playlists(&delete(&[37], if caller { None } else { Some("A") }))
                    .await
            });
            for _ in 0..n {
                f.seen.recv().await.unwrap();
            }
            let replacement = credential("111", "new-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                    Some(replacement.clone());
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut e = task.await.unwrap().unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict);
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(e.take_caller_credential_update().is_none());
            assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
            assert_eq!(f.requests.await.unwrap().len(), n);
        }
    }
}

#[tokio::test]
async fn management_inputs_validate_all_refs_fields_and_owner_before_network() {
    let mut f = server(vec![]).await;
    store_account(&mut f.provider);
    for text in ["", " ", "bad\0name"] {
        let mut r = create(Some("A"));
        r.name = text.into();
        assert_eq!(
            f.provider.create_playlist(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let mut r = create(Some("A"));
    r.kind = PlaylistKind::Video;
    assert!(f.provider.create_playlist(&r).await.is_err());
    r.kind = PlaylistKind::Normal;
    r.visibility = PlaylistVisibility::PlatformDefault;
    assert!(f.provider.create_playlist(&r).await.is_err());
    for ids in [vec![], vec![37, 37], (1..=101).collect()] {
        assert_eq!(
            f.provider
                .delete_playlists(&delete(&ids, Some("A")))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .update_playlist("cloudlist:222:0:37", &update(Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let mut r = update(Some("A"));
    r.tags = Some(vec!["comma,value".into()]);
    assert_eq!(
        f.provider.update_playlist(REF, &r).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        f.provider
            .set_playlist_subscription("cloudlist:111:0:37", false, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn unrelated_directory_sort_may_be_renumbered_but_never_reordered_by_a_management_write() {
    for reversed in [false, true] {
        let mut a = row(3);
        a["sort"] = json!(3);
        let mut b = row(4);
        b["sort"] = json!(7);
        let mut frames = start();
        frames.push(library(vec![row(37), a.clone(), b.clone()], 9).into());
        frames.push(ack(37, 0, 9, 10, 2));
        a["sort"] = json!(if reversed { 20 } else { 10 });
        b["sort"] = json!(if reversed { 10 } else { 20 });
        frames.push(library(vec![a, b], 10).into());
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let result = f.provider.delete_playlists(&delete(&[37], Some("A"))).await;
        if reversed {
            let e = result.unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict);
            assert_eq!(e.details["write_outcome"], "unconfirmed");
        } else {
            assert_eq!(result.unwrap().playlist_refs.len(), 1);
        }
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn management_auth_failures_suppress_updates_while_other_failures_preserve_rotation() {
    for caller in [false, true] {
        for auth in [false, true] {
            for mode in ["create", "modify", "delete", "uncollect"] {
                let mut frames = start();
                frames.push(
                    library(
                        vec![if mode == "uncollect" {
                            collection(37)
                        } else {
                            row(37)
                        }],
                        9,
                    )
                    .into(),
                );
                frames.push(raw(json!({"status":0,"error_code":if auth {20017} else {20010},"data":"management-business-marker"})).into());
                let mut f = server(frames).await;
                let store = store_account(&mut f.provider);
                let saved = read(&store, "A");
                let other = read(&store, "B");
                let provider = if caller {
                    f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
                } else {
                    f.provider.clone()
                };
                let account = if caller { None } else { Some("A") };
                let mut e = match mode {
                    "create" => provider
                        .create_playlist(&create(account))
                        .await
                        .err()
                        .unwrap(),
                    "modify" => provider
                        .update_playlist(REF, &update(account))
                        .await
                        .err()
                        .unwrap(),
                    "delete" => provider
                        .delete_playlists(&delete(&[37], account))
                        .await
                        .err()
                        .unwrap(),
                    _ => provider
                        .set_playlist_subscription(GID, false, account)
                        .await
                        .err()
                        .unwrap(),
                };
                assert_eq!(
                    e.code,
                    if auth {
                        ErrorCode::AuthenticationRequired
                    } else {
                        ErrorCode::UpstreamError
                    }
                );
                assert_eq!(e.details["write_outcome"], "unconfirmed");
                assert_eq!(e.details["write_requests_dispatched"], 1);
                assert!(!e.retryable);
                assert_eq!(e.take_caller_credential_update().is_some(), caller && !auth);
                assert!(!format!("{e:?}").contains("management-business-marker"));
                assert_eq!(read(&store, "B"), other);
                if caller {
                    assert_eq!(read(&store, "A"), saved);
                }
                assert_eq!(f.requests.await.unwrap().len(), 4);
            }
        }
    }
}
