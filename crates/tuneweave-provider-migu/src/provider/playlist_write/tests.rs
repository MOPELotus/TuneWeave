use super::*;
use crate::client::account_playlist::tests::{home, metadata, page};
use crate::provider::{
    favorites::tests::server,
    session::tests::{Store, gated, read, stored},
};
use std::time::Duration;

const TITLE: &str = "测试 A & B # / 100%";
mod cover;
mod metadata;
mod order;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Create,
    Rename,
    Delete,
    Add,
    Remove,
}
const KINDS: [Kind; 5] = [
    Kind::Create,
    Kind::Rename,
    Kind::Delete,
    Kind::Add,
    Kind::Remove,
];
struct Flow {
    values: Vec<serde_json::Value>,
    labels: Vec<&'static str>,
}
impl Flow {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            labels: Vec::new(),
        }
    }
    fn push(&mut self, label: &'static str, value: serde_json::Value) {
        self.labels.push(label);
        self.values.push(value);
    }
    fn profile(&mut self) {
        self.push(
            "profile",
            json!({"code":"000000","data":{"userId":"111","nickName":"Account"}}),
        );
    }
    fn pair(&mut self, label: &'static str, value: serde_json::Value) {
        self.push(label, value);
        self.profile();
    }
    fn data(&mut self, label: &'static str, value: serde_json::Value) {
        self.pair(label, json!({"code":"000000","data":value}));
    }
    fn library(&mut self, label: &'static str, ids: &[u32], renamed: bool) {
        for chunk in ids.chunks(20) {
            self.pair(label,json!({"code":"000000","totalCount":ids.len(),"list":chunk.iter().map(|id|json!({
                "musicListId":id.to_string(),"title":if *id==77 && renamed {TITLE.to_owned()} else {format!("Playlist {id}")},
                "musicNum":0,"ownerId":"111","ownerName":"Account"
            })).collect::<Vec<_>>()}));
        }
        if ids.is_empty() {
            self.pair(label, json!({"code":"000000","totalCount":0,"list":[]}));
        }
    }
    fn snapshot(&mut self, after: bool, ids: &[u32]) {
        self.profile();
        self.data(
            if after {
                "after_metadata"
            } else {
                "before_metadata"
            },
            metadata("77", "111", ids.len()),
        );
        for chunk in ids.chunks(50) {
            self.data(
                if after {
                    "after_tracks"
                } else {
                    "before_tracks"
                },
                page(chunk, ids.len()),
            );
        }
        if ids.is_empty() {
            self.data(
                if after {
                    "after_tracks"
                } else {
                    "before_tracks"
                },
                page(&[], 0),
            );
        }
        self.data(
            if after {
                "after_metadata_final"
            } else {
                "before_metadata_final"
            },
            metadata("77", "111", ids.len()),
        );
    }
    fn at(&self, label: &str) -> usize {
        self.labels.iter().position(|name| *name == label).unwrap()
    }
    fn wire(&self) -> Vec<String> {
        self.values.iter().enumerate().map(|(i,v)|{
            let body=v.to_string();format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: p{i}\r\nConnection: close\r\n\r\n{body}",body.len())
        }).collect()
    }
    fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
        self.labels.truncate(len);
    }
}
fn frames(kind: Kind) -> Flow {
    frames_with_after(kind, None)
}
fn frames_with_after(kind: Kind, override_after: Option<&[u32]>) -> Flow {
    let mut f = Flow::new();
    f.profile();
    if kind != Kind::Create {
        f.data("before_home", home("999"));
    }
    let before: Vec<u32> = if kind == Kind::Create {
        (1..=21).collect()
    } else {
        std::iter::once(77).chain(1..=20).collect()
    };
    f.library("before_library", &before, false);
    if matches!(kind, Kind::Rename | Kind::Delete) {
        f.data("before_metadata", metadata("77", "111", 0));
    }
    // A duplicate original entry and a second physical page must survive edits.
    let old: Vec<u32> = (1..=50).chain([1]).collect();
    if matches!(kind, Kind::Add | Kind::Remove) {
        f.snapshot(false, &old);
    }
    f.pair(
        "write",
        if kind == Kind::Create {
            json!({"code":"000000"})
        } else {
            json!({"code":"000000","musicListId":"77"})
        },
    );
    if matches!(kind, Kind::Add | Kind::Remove) {
        let new: Vec<u32> = if let Some(after) = override_after {
            after.to_vec()
        } else if kind == Kind::Add {
            old.iter().copied().chain([101, 102]).collect()
        } else {
            old.iter()
                .copied()
                .filter(|id| *id != 1 && *id != 2)
                .collect()
        };
        f.snapshot(true, &new);
    }
    let after: Vec<u32> = match kind {
        Kind::Create => std::iter::once(77).chain(before.iter().copied()).collect(),
        Kind::Delete => before.iter().copied().filter(|id| *id != 77).collect(),
        _ => before,
    };
    f.library(
        "after_library",
        &after,
        matches!(kind, Kind::Create | Kind::Rename),
    );
    if matches!(kind, Kind::Create | Kind::Rename) {
        let mut value = metadata("77", "111", 0);
        value["title"] = json!(TITLE);
        f.data("after_metadata", value);
    }
    if kind != Kind::Create {
        f.data("after_home", home("999"));
    }
    f
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
fn reference(id: &str) -> ResourceRef {
    ResourceRef::new(Platform::Migu, id).unwrap()
}
async fn run(p: &MiguProvider, kind: Kind, account: Option<&str>) -> Result<serde_json::Value> {
    let account = account.map(str::to_owned);
    match kind {
        Kind::Create => p
            .create_playlist(&PlaylistCreateRequest {
                name: TITLE.into(),
                visibility: PlaylistVisibility::PlatformDefault,
                kind: PlaylistKind::Normal,
                account,
            })
            .await
            .map(|v| json!(v)),
        Kind::Rename => p
            .update_playlist(
                "77",
                &PlaylistUpdateRequest {
                    name: Some(TITLE.into()),
                    account,
                    ..Default::default()
                },
            )
            .await
            .map(|v| json!(v)),
        Kind::Delete => p
            .delete_playlists(&PlaylistDeleteRequest {
                playlist_refs: vec![reference("77")],
                account,
            })
            .await
            .map(|v| json!(v)),
        Kind::Add | Kind::Remove => p
            .mutate_playlist_items(
                "77",
                if kind == Kind::Add {
                    PlaylistItemMutationAction::Add
                } else {
                    PlaylistItemMutationAction::Remove
                },
                &PlaylistItemMutationRequest {
                    item_refs: if kind == Kind::Add {
                        vec![reference("101"), reference("102")]
                    } else {
                        vec![reference("1"), reference("2")]
                    },
                    kind: PlaylistItemKind::Track,
                    account,
                },
            )
            .await
            .map(|v| json!(v)),
    }
}
fn is_write(request: &str) -> bool {
    request.starts_with("POST ") || request.starts_with("GET /pc/v1.0/user/deleteMusicList.do?")
}

#[tokio::test]
async fn owned_playlist_writes_verify_exact_requests_and_full_readback_for_both_sources() {
    for kind in KINDS {
        for caller in [false, true] {
            let f = frames(kind);
            let (mut p, requests) = server(f.wire()).await;
            let (store, a, b) = setup(&mut p);
            let alias = if caller {
                p = p.caller_scope(&a.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let result = run(&p, kind, Some(alias)).await.unwrap();
            assert_eq!(result["extensions"]["source_user_id"], "111");
            assert!(!result.to_string().contains("initial"));
            assert!(!result.to_string().contains("do-not-export"));
            if matches!(kind, Kind::Add | Kind::Remove) {
                assert!(
                    result["snapshot_id"]
                        .as_str()
                        .unwrap()
                        .starts_with("migu_account_playlist_v1_")
                );
            }
            if kind == Kind::Create {
                assert_eq!(result["playlist_ref"], "migu:77");
                assert_eq!(result["extensions"]["visibility"], "platform_default");
            }
            let update = p.take_response_credential().unwrap();
            if caller {
                assert_eq!(
                    MiguCredential::parse_caller(&update.unwrap())
                        .unwrap()
                        .token(),
                    format!("p{}", f.values.len() - 1)
                );
                assert_eq!(read(&store, "A"), a);
            } else {
                assert!(update.is_none());
                assert_eq!(
                    read(&store, "A").token(),
                    format!("p{}", f.values.len() - 1)
                );
            }
            assert_eq!(read(&store, "B"), b);
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), f.values.len());
            if kind == Kind::Create {
                assert!(
                    !requests
                        .iter()
                        .any(|request| request.contains("/pc/user/home-page/v2.0"))
                );
            }
            let writes: Vec<_> = requests.iter().filter(|r| is_write(r)).collect();
            assert_eq!(writes.len(), 1);
            let write = writes[0];
            if kind == Kind::Delete {
                assert!(
                    write.starts_with("GET /pc/v1.0/user/deleteMusicList.do?channel=23&id=77 ")
                );
            } else {
                let body: serde_json::Value =
                    serde_json::from_str(write.split_once("\r\n\r\n").unwrap().1).unwrap();
                let (path, expected) = match kind {
                    Kind::Create => (
                        "/pc/open/api/music-list/add/v2.0",
                        json!({"title":TITLE,"channel":"23","type":"self_build"}),
                    ),
                    Kind::Rename => (
                        "/pc/user/h5-import-musiclist/v1.0",
                        json!({"title":TITLE,"channel":"23","id":"77","songflag":"0"}),
                    ),
                    Kind::Add => (
                        "/pc/user/api/add-music-list-song/v1.0",
                        json!({"id":"77","contentIds":["101","102"]}),
                    ),
                    Kind::Remove => (
                        "/pc/user/h5-import-musiclist/v1.0",
                        json!({"channel":"23","id":"77","songflag":"2","contentId":"1|2"}),
                    ),
                    _ => unreachable!(),
                };
                assert_eq!(body, expected);
                assert!(write.starts_with(&format!("POST {path} ")));
            }
            for (i, r) in requests.iter().enumerate() {
                let token = if i == 0 {
                    "initial".into()
                } else {
                    format!("p{}", i - 1)
                };
                if is_write(r) {
                    assert!(
                        r.contains(&format!("cookie: pacmtoken={token}\r\n")),
                        "{kind:?} {i}"
                    );
                    assert!(!r.contains(&format!("pacmtoken: {token}\r\n")));
                } else {
                    assert!(
                        r.contains(&format!("pacmtoken: {token}\r\n")),
                        "{kind:?} {i}"
                    );
                    assert!(!r.contains("cookie:"));
                }
                assert!(!r.contains("other"));
            }
        }
    }
}

#[tokio::test]
async fn ordinary_writes_allow_missing_favorite_navigation_with_complete_owned_library_proof() {
    for kind in KINDS {
        let mut f = frames(kind);
        for label in ["before_home", "after_home"] {
            if let Some(index) = f.labels.iter().position(|name| *name == label) {
                f.values[index]["data"]["userPrivateItems"][1]["actionUrl"] = json!("");
            }
        }
        let (mut p, requests) = server(f.wire()).await;
        setup(&mut p);
        run(&p, kind, Some("A")).await.unwrap();
        assert_eq!(
            requests
                .await
                .unwrap()
                .iter()
                .filter(|request| is_write(request))
                .count(),
            1,
            "{kind:?}"
        );
    }
}

#[tokio::test]
async fn playlist_mutations_reject_unsupported_inputs_before_network_or_account_selection() {
    let (p, requests) = server(vec![]).await;
    for visibility in [PlaylistVisibility::Public, PlaylistVisibility::Private] {
        let mut r = PlaylistCreateRequest::new(TITLE);
        r.visibility = visibility;
        assert_eq!(
            p.create_playlist(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for title in ["", " ", "a\nb"] {
        let r = PlaylistCreateRequest {
            name: title.into(),
            visibility: PlaylistVisibility::PlatformDefault,
            kind: PlaylistKind::Normal,
            account: None,
        };
        assert_eq!(
            p.create_playlist(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for kind in [PlaylistKind::Video, PlaylistKind::Shared] {
        let r = PlaylistCreateRequest {
            name: TITLE.into(),
            visibility: PlaylistVisibility::PlatformDefault,
            kind,
            account: None,
        };
        assert_eq!(
            p.create_playlist(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for variant in 0..4 {
        let mut r = PlaylistUpdateRequest {
            name: Some(TITLE.into()),
            ..Default::default()
        };
        match variant {
            0 => r.description = Some("".into()),
            1 => r.tags = Some(vec!["duplicate".into(), "duplicate".into()]),
            2 => r.variant = PlaylistMetadataUpdateVariant::Batch,
            _ => r.name = None,
        };
        assert_eq!(
            p.update_playlist("77", &r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for values in [
        vec![],
        vec![reference("77"); 2],
        vec![ResourceRef::new(Platform::Qq, "77").unwrap()],
        vec![reference("077")],
        (1..=101).map(|id| reference(&id.to_string())).collect(),
    ] {
        assert_eq!(
            p.delete_playlists(&PlaylistDeleteRequest {
                playlist_refs: values,
                account: None
            })
            .await
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        p.mutate_playlist_items(
            "77",
            PlaylistItemMutationAction::Add,
            &PlaylistItemMutationRequest::new(vec![reference("1")], PlaylistItemKind::Video)
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    for values in [vec![], vec![reference("1"); 2], vec![reference("1|2")]] {
        assert_eq!(
            p.mutate_playlist_items(
                "77",
                PlaylistItemMutationAction::Add,
                &PlaylistItemMutationRequest::new(values, PlaylistItemKind::Track)
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn existing_additions_and_absent_removals_require_unchanged_duplicate_positions() {
    let old: Vec<u32> = (1..=50).chain([1]).collect();
    for (kind, action, selected) in [
        (Kind::Add, PlaylistItemMutationAction::Add, ["1", "2"]),
        (
            Kind::Remove,
            PlaylistItemMutationAction::Remove,
            ["101", "102"],
        ),
    ] {
        let f = frames_with_after(kind, Some(&old));
        let (mut p, requests) = server(f.wire()).await;
        setup(&mut p);
        let result = p
            .mutate_playlist_items(
                "77",
                action,
                &PlaylistItemMutationRequest {
                    item_refs: selected.map(reference).to_vec(),
                    kind: PlaylistItemKind::Track,
                    account: Some("A".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(result.item_refs, selected.map(reference).to_vec());
        assert_eq!(result.extensions["existing_track_order_preserved"], true);
        assert_eq!(
            requests
                .await
                .unwrap()
                .iter()
                .filter(|request| is_write(request))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn mutations_refuse_favorites_noncreated_playlists_and_foreign_owners_before_writing() {
    for kind in [Kind::Rename, Kind::Delete, Kind::Add, Kind::Remove] {
        for variant in 0..3 {
            let mut f = frames(kind);
            if variant == 0 {
                let i = f.at("before_home");
                f.values[i]["data"] = home("77");
                f.truncate(
                    f.at("before_metadata") - usize::from(matches!(kind, Kind::Add | Kind::Remove)),
                );
            } else if variant == 1 {
                let i = f.at("before_library");
                f.values[i]["list"][0]["musicListId"] = json!("88");
                f.truncate(
                    f.at("before_metadata") - usize::from(matches!(kind, Kind::Add | Kind::Remove)),
                );
            } else {
                for i in 0..f.values.len() {
                    if matches!(
                        f.labels[i],
                        "before_metadata" | "before_metadata_final" | "before_tracks"
                    ) {
                        f.values[i]["data"]["ownerId"] = json!("222");
                    }
                }
                f.truncate(f.at("write"));
            }
            let (mut p, requests) = server(f.wire()).await;
            setup(&mut p);
            let e = run(&p, kind, Some("A")).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::PermissionDenied, "{kind:?} {variant}");
            assert!(e.details.get("write_outcome").is_none());
            assert!(!requests.await.unwrap().iter().any(|r| is_write(r)));
        }
    }
}

#[tokio::test]
async fn mutation_acknowledgements_never_replace_confirmation_or_retry_failed_writes() {
    for kind in KINDS {
        for variant in 0..5 {
            let mut f = frames(kind);
            let write = f.at("write");
            f.values[write] = match variant {
                0 => json!({"code":"299999"}),
                1 => json!({"code":"290001"}),
                2 => json!({"code":"000000","userId":"222"}),
                3 => json!({"code":"000000","musicListId":"078"}),
                _ => json!({"code":"000000","musicListId":"88"}),
            };
            let count = if kind == Kind::Create && variant == 4 {
                f.at("after_metadata")
            } else {
                write + if variant < 3 { 1 } else { 2 }
            };
            f.truncate(count);
            let (mut p, requests) = server(f.wire()).await;
            let (store, a, b) = setup(&mut p);
            let p = p.caller_scope(&a.caller().unwrap()).unwrap();
            let mut e = run(&p, kind, None).await.unwrap_err();
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            let auth = matches!(variant, 1 | 2);
            assert_eq!(
                e.code,
                if auth {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            let update = e.take_caller_credential_update();
            assert_eq!(update.is_none(), auth);
            if let Some(update) = update {
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    format!("p{}", if variant < 3 { write - 1 } else { count - 1 })
                );
            }
            assert_eq!(read(&store, "A"), a);
            assert_eq!(read(&store, "B"), b);
            assert_eq!(
                requests
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| is_write(r))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn readback_rejects_ambiguous_creation_incomplete_later_pages_and_false_mutation_success() {
    for kind in KINDS {
        let variants = if kind == Kind::Create { 2 } else { 3 };
        for variant in 0..variants {
            let mut f = frames(kind);
            let end;
            if variant == 2 {
                let i = f.at("after_home");
                f.values[i]["data"] = home("888");
                end = i + 2;
            } else {
                match kind {
                    Kind::Create | Kind::Rename => {
                        let i = f.at("after_library");
                        if variant == 0 {
                            f.values[i]["list"][0]["title"] = json!("Wrong name");
                        } else {
                            f.values[i + 2]["list"][0]["musicListId"] = json!("777");
                        }
                        end = f.at("after_metadata");
                    }
                    Kind::Delete => {
                        let i = f.at("after_library");
                        f.values[i]["list"][0]["musicListId"] =
                            json!(if variant == 0 { "77" } else { "666" });
                        end = i + 2;
                    }
                    Kind::Add | Kind::Remove => {
                        let i = f.at("after_tracks");
                        if variant == 0 {
                            f.values[i]["data"]["songList"]
                                .as_array_mut()
                                .unwrap()
                                .swap(0, 1);
                        } else {
                            f.values[i]["data"]["songList"][0]["contentId"] =
                                json!(if kind == Kind::Add { "101" } else { "1" });
                        }
                        end = f.at("after_library");
                    }
                }
            }
            f.truncate(end);
            let (mut p, requests) = server(f.wire()).await;
            let (store, a, b) = setup(&mut p);
            let p = p.caller_scope(&a.caller().unwrap()).unwrap();
            let mut e = run(&p, kind, None).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::UpstreamError, "{kind:?} {variant}");
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            assert_eq!(
                MiguCredential::parse_caller(&e.take_caller_credential_update().unwrap())
                    .unwrap()
                    .token(),
                format!("p{}", end - 1)
            );
            assert_eq!(read(&store, "A"), a);
            assert_eq!(read(&store, "B"), b);
            assert_eq!(
                requests
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| is_write(r))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn deletion_batches_prevalidate_every_owner_and_preserve_confirmed_prefix_on_failure() {
    for invalid_owner in [false, true] {
        let mut f = Flow::new();
        f.profile();
        f.data("home", home("999"));
        f.library("before", &[77, 78, 79], false);
        f.data("metadata", metadata("77", "111", 0));
        f.data(
            "metadata",
            metadata("78", if invalid_owner { "222" } else { "111" }, 0),
        );
        if !invalid_owner {
            f.data("metadata", metadata("79", "111", 0));
            f.pair("write", json!({"code":"000000"}));
            f.library("after", &[78, 79], false);
            f.data("home", home("999"));
            f.push("write", json!({"code":"299999"}));
        }
        let (mut p, requests) = server(f.wire()).await;
        setup(&mut p);
        let e = p
            .delete_playlists(&PlaylistDeleteRequest {
                playlist_refs: vec![reference("77"), reference("78"), reference("79")],
                account: Some("A".into()),
            })
            .await
            .unwrap_err();
        if invalid_owner {
            assert_eq!(e.code, ErrorCode::PermissionDenied);
            assert!(e.details.get("write_outcome").is_none());
        } else {
            assert_eq!(e.details["atomic"], false);
            assert_eq!(e.details["completed_refs"], json!(["migu:77"]));
            assert_eq!(e.details["failed_ref"], "migu:78");
            assert_eq!(e.details["remaining_refs"], json!(["migu:79"]));
            assert!(!e.retryable);
        }
        assert_eq!(
            requests
                .await
                .unwrap()
                .iter()
                .filter(|r| is_write(r))
                .count(),
            if invalid_owner { 0 } else { 2 }
        );
    }
}

#[tokio::test]
async fn every_playlist_write_boundary_rejects_relogin_or_logout_without_exporting_old_credentials()
{
    for kind in KINDS {
        let f = frames(kind);
        let replies = f.wire();
        for caller in [false, true] {
            for stage in 1..=replies.len() {
                let (mut p, seen, release, server) = gated(replies[..stage].to_vec()).await;
                let (store, a, b) = setup(&mut p);
                let alias = if caller {
                    p = p.caller_scope(&a.caller().unwrap()).unwrap();
                    "default"
                } else {
                    "A"
                };
                let p = Arc::new(p);
                let worker = p.clone();
                let task = tokio::spawn(async move { run(&worker, kind, Some(alias)).await });
                tokio::time::timeout(Duration::from_secs(5), seen)
                    .await
                    .unwrap()
                    .unwrap();
                let newer = MiguCredential::verified("111".into(), "new-login".into()).unwrap();
                if caller {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() = newer.clone();
                } else if stage % 2 == 0 {
                    store.remove(Platform::Migu, "A").unwrap();
                } else {
                    store.put(&stored("A", &newer)).unwrap();
                }
                release.send(()).unwrap();
                let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(e.code, ErrorCode::Conflict, "{kind:?} {stage}");
                assert!(e.take_caller_credential_update().is_none());
                assert!(p.take_response_credential().unwrap().is_none());
                assert_eq!(
                    e.details.get("write_outcome").is_some(),
                    stage > f.at("write")
                );
                if caller {
                    assert_eq!(read(&store, "A"), a);
                } else if stage % 2 != 0 {
                    assert_eq!(read(&store, "A"), newer);
                } else {
                    assert!(
                        !store
                            .load_platform(Platform::Migu)
                            .unwrap()
                            .iter()
                            .any(|v| v.account == "A")
                    );
                }
                assert_eq!(read(&store, "B"), b);
                server.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn playlist_write_timeouts_preserve_only_prior_identity_verified_rotations() {
    for kind in KINDS {
        let f = frames(kind);
        let replies = f.wire();
        for stage in 1..=replies.len() {
            let (mut p, seen, release, server) = gated(replies[..stage].to_vec()).await;
            let (_, a, _) = setup(&mut p);
            p.client = p
                .client
                .with_session_test_timeout(Duration::from_millis(200));
            let p = Arc::new(p.caller_scope(&a.caller().unwrap()).unwrap());
            let worker = p.clone();
            let task = tokio::spawn(async move { run(&worker, kind, None).await });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::UpstreamTimeout, "{kind:?} {stage}");
            assert_eq!(
                e.details.get("write_outcome").is_some(),
                stage > f.at("write")
            );
            let last = f.labels[..stage - 1]
                .iter()
                .rposition(|label| *label == "profile");
            let update = e.take_caller_credential_update();
            assert_eq!(update.is_some(), last.is_some());
            if let (Some(update), Some(index)) = (update, last) {
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    format!("p{index}")
                );
            }
            assert_eq!(
                p.take_response_credential()
                    .unwrap()
                    .map(|value| MiguCredential::parse_caller(&value)
                        .unwrap()
                        .token()
                        .to_owned()),
                last.map(|index| format!("p{index}")),
            );
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
            drop(release);
        }
    }
}
