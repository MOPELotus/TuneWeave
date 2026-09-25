use super::*;
use crate::provider::account_media::tests::setup as setup_mode;
use std::collections::BTreeMap;

const DESCRIPTION: &str = "测试简介 & x=y + 100%\n第二行 / # ";
const OLD_TITLE: &str = "Actual playlist name";
const NATIVE_PATH: &str = "/MIGUM2.0/v1.0/user/updateMusicList.do";

mod tag_order;

fn flow(rename: bool) -> Flow {
    let mut f = Flow::new();
    f.profile();
    f.data("before_home", home("999"));
    let created: Vec<_> = std::iter::once(77).chain(1..=20).collect();
    f.library("before_library", &created, false);
    let tracks: Vec<_> = (1..=50).chain([1]).collect();
    f.snapshot(false, &tracks);
    f.push("native_profile", json!({"code":"000000","data":{"userId":"111","nickName":"Account","usessionId":"native-session-fixture"}}));
    f.pair(
        "exchange",
        json!({"code":"000000","data":"native-token-fixture"}),
    );
    f.push(
        "validate",
        json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}}),
    );
    f.push("write", json!({"code":"000000"}));
    f.snapshot(true, &tracks);
    f.library("after_library", &created, rename);
    f.data("after_home", home("999"));
    for (label, value) in f.labels.iter().zip(&mut f.values) {
        if matches!(*label, "after_metadata" | "after_metadata_final") {
            value["data"]["summary"] = json!(DESCRIPTION);
            value["data"]["title"] = json!(if rename { TITLE } else { OLD_TITLE });
        }
        if *label == "after_library" && !rename {
            for entry in value["list"].as_array_mut().unwrap() {
                if entry["musicListId"] == "77" {
                    entry["title"] = json!(OLD_TITLE);
                }
            }
        }
    }
    f
}
fn tag_value(tags: &[(&str, &str)]) -> serde_json::Value {
    json!(
        tags.iter()
            .map(|(id, name)| json!({"tagId":id,"tagName":name}))
            .collect::<Vec<_>>()
    )
}
fn tag_flow(existing: &[(&str, &str)], after_each_change: &[&[(&str, &str)]]) -> Flow {
    let mut f = Flow::new();
    f.profile();
    f.data("before_home", home("999"));
    let created: Vec<_> = std::iter::once(77).chain(1..=20).collect();
    f.library("before_library", &created, false);
    let tracks: Vec<_> = (1..=50).chain([1]).collect();
    let start = f.values.len();
    f.snapshot(false, &tracks);
    for (index, label) in f.labels.iter().enumerate().skip(start) {
        if matches!(*label, "before_metadata" | "before_metadata_final") {
            f.values[index]["data"]["tags"] = tag_value(existing);
        }
    }
    f.push(
        "native_profile",
        json!({"code":"000000","data":{"userId":"111","nickName":"Account","usessionId":"native-session-fixture"}}),
    );
    f.pair(
        "exchange",
        json!({"code":"000000","data":"native-token-fixture"}),
    );
    f.push(
        "validate",
        json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}}),
    );
    for tags in after_each_change {
        f.push("write", json!({"code":"000000"}));
        let start = f.values.len();
        f.snapshot(true, &tracks);
        for (index, label) in f.labels.iter().enumerate().skip(start) {
            if matches!(*label, "after_metadata" | "after_metadata_final") {
                f.values[index]["data"]["tags"] = tag_value(tags);
            }
        }
    }
    f.library("after_library", &created, false);
    for (label, value) in f.labels.iter().zip(&mut f.values) {
        if *label == "after_library" {
            for entry in value["list"].as_array_mut().unwrap() {
                if entry["musicListId"] == "77" {
                    entry["title"] = json!(OLD_TITLE);
                }
            }
        }
    }
    f.data("after_home", home("999"));
    f
}
fn with_tag_catalogue(mut f: Flow, tags: &[(&str, &str)]) -> Flow {
    let at = f.at("native_profile");
    f.labels.insert(at, "tag_catalogue");
    f.values.insert(at, json!({"code":"000000","data":[{"content":tags.iter().map(|(id,name)|json!({"texts":[name,id,"display only"]})).collect::<Vec<_>>()}]}));
    f
}
fn wire(f: &Flow) -> Vec<String> {
    let mut replies = f.wire();
    if let Some(at) = f.labels.iter().position(|label| *label == "validate") {
        let body = crate::client::native_http::encode(f.values[at].to_string().as_bytes());
        replies[at] = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
    replies
}
fn request(
    alias: &str,
    rename: bool,
    variant: PlaylistMetadataUpdateVariant,
) -> PlaylistUpdateRequest {
    PlaylistUpdateRequest {
        account: Some(alias.into()),
        description: Some(DESCRIPTION.into()),
        name: rename.then(|| TITLE.into()),
        variant,
        ..Default::default()
    }
}
async fn run_metadata(
    p: &MiguProvider,
    alias: &str,
    rename: bool,
    variant: PlaylistMetadataUpdateVariant,
) -> Result<PlaylistMutationResult> {
    p.update_playlist("77", &request(alias, rename, variant))
        .await
}

#[tokio::test]
async fn native_playlist_metadata_uses_form_and_selected_identity_preserving_duplicate_track_order()
{
    for mode in ["default", "named", "caller"] {
        for (rename, variant) in [
            (false, PlaylistMetadataUpdateVariant::Individual),
            (true, PlaylistMetadataUpdateVariant::Batch),
            (true, PlaylistMetadataUpdateVariant::Default),
        ] {
            let f = flow(rename);
            let (mut p, seen) = server(wire(&f)).await;
            let (store, original, alias) = setup_mode(&mut p, mode);
            let result = run_metadata(&p, alias, rename, variant).await.unwrap();
            assert_eq!(result.playlist.as_ref().unwrap().description, DESCRIPTION);
            assert_eq!(
                result.playlist.as_ref().unwrap().name,
                if rename { TITLE } else { OLD_TITLE }
            );
            assert_eq!(result.extensions["existing_track_order_preserved"], true);
            let encoded = serde_json::to_string(&result).unwrap();
            for secret in [
                "native-token-fixture",
                "native-session-fixture",
                "do-not-export",
            ] {
                assert!(!encoded.contains(secret));
            }
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                let credential = p.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&credential).unwrap().token(),
                    format!("p{}", f.values.len() - 1)
                );
                assert!(!credential.secret().contains("native-token-fixture"));
                assert!(!credential.secret().contains("native-session-fixture"));
            } else {
                assert_eq!(
                    read(&store, alias).token(),
                    format!("p{}", f.values.len() - 1)
                );
            }
            let requests = seen.await.unwrap();
            assert_eq!(requests.len(), f.values.len());
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.starts_with("POST "))
                    .count(),
                1
            );
            let write = &requests[f.at("write")];
            assert!(write.starts_with(&format!("POST {NATIVE_PATH} HTTP/1.1")));
            assert!(write.contains("content-type: application/x-www-form-urlencoded\r\n"));
            assert!(write.contains("token: native-token-fixture\r\n"));
            assert!(write.contains("signversion: V005\r\n"));
            let body = write.split_once("\r\n\r\n").unwrap().1;
            let fields: BTreeMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect();
            assert_eq!(fields.len(), if rename { 4 } else { 3 });
            assert_eq!(fields["id"], "77");
            assert_eq!(fields["songflag"], "0");
            assert_eq!(fields["info"], DESCRIPTION);
            if rename {
                assert_eq!(fields["title"], TITLE);
            }
            for forbidden in [
                "pacmtoken:",
                "native-session-fixture",
                "cookie:",
                "usessionid",
                "\r\nuid:",
            ] {
                assert!(!write.to_lowercase().contains(forbidden));
            }
            assert!(
                requests[f.at("exchange")]
                    .starts_with("GET /user/h5/token/v1.0?uSessionId=native-session-fixture&_t=")
            );
            assert!(requests[f.at("validate")].starts_with("GET /user/token-validate/v2.0?"));
            assert!(
                !requests
                    .iter()
                    .any(|request| request.contains("h5-import-musiclist"))
            );
        }
    }
}

#[tokio::test]
async fn native_playlist_metadata_blocks_unknown_fields_and_clear_before_selecting_account() {
    let (p, seen) = server(vec![]).await;
    for variant in 0..8 {
        let mut request = request("default", false, PlaylistMetadataUpdateVariant::Default);
        match variant {
            0 => request.description = Some(String::new()),
            1 => request.description = Some(" \n\t".into()),
            2 => request.description = Some("a\0b".into()),
            3 => request.description = Some("界".repeat(1334)),
            4 => request.tags = Some(vec!["duplicate".into(), "duplicate".into()]),
            5 => {
                request.name = Some(TITLE.into());
                request.variant = PlaylistMetadataUpdateVariant::Individual;
            }
            6 => request.name = Some("bad\nname".into()),
            _ => request.tags = Some(vec!["流行|国语".into()]),
        }
        assert_eq!(
            p.update_playlist("77", &request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(seen.await.unwrap().is_empty());
}

#[tokio::test]
async fn native_playlist_tags_can_be_cleared_by_verified_single_tag_updates() {
    const FIRST: (&str, &str) = ("1000001672", "流行");
    const SECOND: (&str, &str) = ("1000001762", "国语");
    let f = tag_flow(&[FIRST, SECOND], &[&[SECOND], &[]]);
    let (mut p, seen) = server(wire(&f)).await;
    let (store, original, alias) = setup_mode(&mut p, "named");
    let result = p
        .update_playlist(
            "77",
            &PlaylistUpdateRequest {
                tags: Some(Vec::new()),
                account: Some(alias.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.playlist.as_ref().unwrap().tags.is_empty());
    assert_eq!(
        result.extensions["confirmed_tag_removals"],
        json!(["流行", "国语"])
    );
    assert_eq!(result.extensions["atomic"], false);
    assert_eq!(
        read(&store, alias).token(),
        format!("p{}", f.values.len() - 1)
    );
    assert_eq!(original.user_id(), "111");

    let requests = seen.await.unwrap();
    let writes = requests
        .iter()
        .filter(|request| request.starts_with(&format!("POST {NATIVE_PATH} ")))
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 2);
    for (write, (id, name)) in writes.iter().zip([FIRST, SECOND]) {
        assert!(write.contains("content-type: application/x-www-form-urlencoded\r\n"));
        let body = write.split_once("\r\n\r\n").unwrap().1;
        let fields = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(fields["id"], "77");
        assert_eq!(fields["songflag"], "0");
        assert_eq!(fields["delTagIds"], id);
        assert_eq!(fields["delTagNames"], name);
        assert!(!fields.contains_key("info"));
    }
}

#[tokio::test]
async fn native_playlist_tag_unknown_additions_are_rejected_before_mutation() {
    const CURRENT: (&str, &str) = ("1000001672", "流行");
    let mut f = with_tag_catalogue(tag_flow(&[CURRENT], &[]), &[CURRENT]);
    f.truncate(f.at("native_profile"));
    let (mut p, seen) = server(wire(&f)).await;
    setup_mode(&mut p, "named");
    let failure = p
        .update_playlist(
            "77",
            &PlaylistUpdateRequest {
                tags: Some(vec!["流行".into(), "摇滚".into()]),
                account: Some("personal".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::InvalidRequest);
    assert!(failure.details.get("write_outcome").is_none());
    let requests = seen.await.unwrap();
    assert!(!requests.iter().any(|request| request.starts_with("POST ")));
}

#[tokio::test]
async fn native_playlist_tag_additions_use_catalogue_ids_and_confirm_each_change_for_selected_account()
 {
    const OLD: (&str, &str) = ("1000001672", "流行");
    const KEEP: (&str, &str) = ("1000001762", "国语");
    const NEW: (&str, &str) = ("1000001679", "摇滚 & + / %");
    for mode in ["default", "named", "caller"] {
        let f = with_tag_catalogue(
            tag_flow(&[OLD, KEEP], &[&[KEEP], &[KEEP, NEW]]),
            &[OLD, KEEP, NEW],
        );
        let (mut p, seen) = server(wire(&f)).await;
        let (store, original, alias) = setup_mode(&mut p, mode);
        let result = p
            .update_playlist(
                "77",
                &PlaylistUpdateRequest {
                    tags: Some(vec![KEEP.1.into(), NEW.1.into()]),
                    account: Some(alias.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.playlist.as_ref().unwrap().tags, [KEEP.1, NEW.1]);
        assert_eq!(result.extensions["confirmed_tag_removals"], json!([OLD.1]));
        assert_eq!(result.extensions["confirmed_tag_additions"], json!([NEW.1]));
        assert_eq!(result.extensions["existing_track_order_preserved"], true);
        assert_eq!(result.extensions["atomic"], false);
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            let credential = p.take_response_credential().unwrap().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&credential).unwrap().token(),
                format!("p{}", f.values.len() - 1)
            );
            assert!(!credential.secret().contains("native-token-fixture"));
        } else {
            assert_eq!(
                read(&store, alias).token(),
                format!("p{}", f.values.len() - 1)
            );
        }
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), f.values.len());
        let catalogue = &requests[f.at("tag_catalogue")];
        assert!(catalogue.starts_with(
            "GET /MIGUM3.0/v1.0/template/musiclistplaza-taglist/release?templateVersion=1 HTTP/1.1"
        ));
        for secret in [
            "pacmtoken:",
            "cookie:",
            "native-token-fixture",
            "native-session-fixture",
        ] {
            assert!(!catalogue.contains(secret));
        }
        // A catalogue response's unrelated PACM header must not rotate this account.
        assert!(
            requests[f.at("native_profile")]
                .contains(&format!("pacmtoken: p{}\r\n", f.at("tag_catalogue") - 1))
        );
        let writes = requests
            .iter()
            .filter(|r| r.starts_with("POST "))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 2);
        for write in &writes {
            assert!(write.starts_with(&format!("POST {NATIVE_PATH} HTTP/1.1")));
            assert!(write.contains("token: native-token-fixture\r\n"));
            assert!(write.contains("signversion: V005\r\n"));
            assert!(!write.contains("pacmtoken:"));
        }
        let body = writes[1].split_once("\r\n\r\n").unwrap().1;
        let fields = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields["id"], "77");
        assert_eq!(fields["songflag"], "0");
        assert_eq!(fields["addTagIds"], format!("{}|", NEW.0));
        assert_eq!(fields["addTagNames"], format!("{}|", NEW.1));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("native-token-fixture")
        );
    }
}

#[tokio::test]
async fn native_playlist_tag_additions_reject_ambiguous_catalogue_and_unsupported_selection_before_write()
 {
    const CURRENT: (&str, &str) = ("1000001672", "流行");
    const NEW: (&str, &str) = ("1000001679", "摇滚");
    for variant in 0..4 {
        let mut f = with_tag_catalogue(tag_flow(&[CURRENT], &[]), &[CURRENT, NEW]);
        let mut desired = vec![CURRENT.1.into(), NEW.1.into()];
        let (count, code) = match variant {
            0 => {
                let at = f.at("tag_catalogue");
                f.values[at]["data"][0]["content"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"texts":[NEW.1,"99"]}));
                (at + 1, ErrorCode::UpstreamError)
            }
            1 => {
                desired = vec![NEW.1.into(), CURRENT.1.into()];
                let at = f.at("tag_catalogue");
                f.values[at]["data"][0]["content"][0]["texts"] = json!([CURRENT.1, "99"]);
                (at + 1, ErrorCode::UpstreamError)
            }
            2 => {
                desired.extend((1..=5).map(|i| format!("tag{i}")));
                (f.at("tag_catalogue"), ErrorCode::InvalidRequest)
            }
            _ => {
                let at = f.at("tag_catalogue");
                f.values[at]["data"][0]["content"][1]["texts"] = json!([NEW.1, CURRENT.0]);
                (at + 1, ErrorCode::UpstreamError)
            }
        };
        f.truncate(count);
        let (mut p, seen) = server(wire(&f)).await;
        setup_mode(&mut p, "named");
        let failure = p
            .update_playlist(
                "77",
                &PlaylistUpdateRequest {
                    tags: Some(desired),
                    account: Some("personal".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(failure.code, code, "variant {variant}");
        assert!(failure.details.get("write_outcome").is_none());
        assert!(
            !seen
                .await
                .unwrap()
                .iter()
                .any(|request| request.starts_with("POST "))
        );
    }
}

#[tokio::test]
async fn native_playlist_tag_additions_require_readback_and_report_only_confirmed_partial_changes()
{
    const FIRST: (&str, &str) = ("1000001672", "流行");
    const SECOND: (&str, &str) = ("1000001762", "国语");
    for stale in [true, false] {
        let mut f = if stale {
            with_tag_catalogue(tag_flow(&[], &[&[]]), &[FIRST])
        } else {
            with_tag_catalogue(
                tag_flow(&[], &[&[FIRST], &[FIRST, SECOND]]),
                &[FIRST, SECOND],
            )
        };
        if stale {
            f.truncate(f.at("after_library"));
        } else {
            let at = f
                .labels
                .iter()
                .enumerate()
                .filter(|(_, label)| **label == "write")
                .nth(1)
                .unwrap()
                .0;
            f.values[at] = json!({"code":"200013","info":"native-token-fixture"});
            f.truncate(at + 1);
        }
        let (mut p, seen) = server(wire(&f)).await;
        setup_mode(&mut p, "named");
        let tags = if stale {
            vec![FIRST.1.into()]
        } else {
            vec![FIRST.1.into(), SECOND.1.into()]
        };
        let failure = p
            .update_playlist(
                "77",
                &PlaylistUpdateRequest {
                    tags: Some(tags),
                    account: Some("personal".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert!(!failure.retryable);
        if stale {
            assert!(failure.details.get("confirmed_tag_additions").is_none());
        } else {
            assert_eq!(failure.details["confirmed_tag_additions"], json!([FIRST.1]));
        }
        assert!(!failure.details.to_string().contains("native-token-fixture"));
        assert_eq!(
            seen.await
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            if stale { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn native_playlist_tag_catalogue_read_preserves_replacement_login_before_authorization() {
    const TAG: (&str, &str) = ("1000001672", "流行");
    let mut f = with_tag_catalogue(tag_flow(&[], &[]), &[TAG]);
    f.truncate(f.at("tag_catalogue") + 1);
    let (mut p, seen, release, requests) = gated(wire(&f)).await;
    let (store, _, alias) = setup_mode(&mut p, "named");
    let task = tokio::spawn(async move {
        p.update_playlist(
            "77",
            &PlaylistUpdateRequest {
                tags: Some(vec![TAG.1.into()]),
                account: Some(alias.into()),
                ..Default::default()
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("tag flow did not reach catalogue")
        .unwrap();
    let replacement = MiguCredential::verified("111".into(), "new-login".into()).unwrap();
    store.put(&stored(alias, &replacement)).unwrap();
    release.send(()).unwrap();
    let failure = task.await.unwrap().unwrap_err();
    assert_eq!(failure.code, ErrorCode::Conflict);
    assert!(failure.details.get("write_outcome").is_none());
    assert_eq!(read(&store, alias), replacement);
    requests.await.unwrap();
}

#[tokio::test]
async fn native_playlist_tag_clear_reports_confirmed_partial_deletions_without_retry() {
    const FIRST: (&str, &str) = ("1000001672", "流行");
    const SECOND: (&str, &str) = ("1000001762", "国语");
    let mut f = tag_flow(&[FIRST, SECOND], &[&[SECOND], &[]]);
    let write_indexes = f
        .labels
        .iter()
        .enumerate()
        .filter_map(|(index, label)| (*label == "write").then_some(index))
        .collect::<Vec<_>>();
    f.values[write_indexes[1]] = json!({"code":"200013","info":"native-token-fixture"});
    f.truncate(write_indexes[1] + 1);
    let (mut p, seen) = server(wire(&f)).await;
    setup_mode(&mut p, "named");
    let failure = p
        .update_playlist(
            "77",
            &PlaylistUpdateRequest {
                tags: Some(Vec::new()),
                account: Some("personal".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(failure.details["write_outcome"], "unconfirmed");
    assert!(!failure.retryable);
    assert_eq!(failure.details["confirmed_tag_removals"], json!(["流行"]));
    assert!(!failure.details.to_string().contains("native-token-fixture"));
    let requests = seen.await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with(&format!("POST {NATIVE_PATH} ")))
            .count(),
        2
    );
}

#[tokio::test]
async fn native_playlist_metadata_rejects_foreign_favorite_or_unbridged_identity_before_write() {
    for variant in 0..7 {
        let mut f = flow(false);
        let (at, count) = match variant {
            0 => {
                let at = f.at("before_home");
                f.values[at]["data"] = home("77");
                (at, f.at("before_metadata") - 1)
            }
            1 => {
                for i in [f.at("before_metadata"), f.at("before_metadata_final")] {
                    f.values[i]["data"]["ownerId"] = json!("222");
                }
                for (label, value) in f.labels.iter().zip(&mut f.values) {
                    if *label == "before_tracks" {
                        value["data"]["ownerId"] = json!("222");
                    }
                }
                (f.at("before_metadata"), f.at("native_profile"))
            }
            2 => {
                let at = f.at("native_profile");
                f.values[at]["data"]
                    .as_object_mut()
                    .unwrap()
                    .remove("usessionId");
                (at, at + 1)
            }
            3 => {
                let at = f.at("exchange");
                f.values[at]["data"] = json!({"token":"native-token-fixture"});
                (at, at + 2)
            }
            4 => {
                let at = f.at("validate");
                f.values[at]["data"]["userInfoItem"]["userId"] = json!("222");
                (at, at + 1)
            }
            5 => {
                let at = f.at("validate");
                f.values[at]["code"] = json!("200010");
                (at, at + 1)
            }
            _ => {
                let at = f.at("native_profile");
                f.values[at]["data"]["userId"] = json!("222");
                (at, at + 1)
            }
        };
        f.truncate(count);
        let (mut p, seen) = server(wire(&f)).await;
        setup_mode(&mut p, "named");
        let failure = run_metadata(
            &p,
            "personal",
            false,
            PlaylistMetadataUpdateVariant::Default,
        )
        .await
        .unwrap_err();
        assert!(
            failure.details.get("write_outcome").is_none(),
            "variant {variant}, at {at}"
        );
        assert!(!format!("{failure:?}").contains("native-token-fixture"));
        assert!(
            !seen
                .await
                .unwrap()
                .iter()
                .any(|request| request.starts_with("POST "))
        );
    }
}

#[tokio::test]
async fn native_playlist_metadata_never_accepts_ack_without_complete_unchanged_readback() {
    for variant in 0..7 {
        let mut f = flow(true);
        let count = match variant {
            0 => {
                let at = f.at("write");
                f.values[at] = json!({"code":"300102","info":"native-token-fixture"});
                at + 1
            }
            1 => {
                for at in [f.at("after_metadata"), f.at("after_metadata_final")] {
                    f.values[at]["data"]["summary"] = json!("Description");
                }
                f.at("after_library")
            }
            2 => {
                let at = f.at("after_tracks");
                f.values[at]["data"]["songList"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 1);
                f.at("after_library")
            }
            3 => {
                for at in [f.at("after_metadata"), f.at("after_metadata_final")] {
                    f.values[at]["data"]["originalImgUrl"] =
                        json!("https://d.musicapp.migu.cn/data/oss/changed.png");
                }
                f.at("after_library")
            }
            4 => {
                let at = f.at("after_library");
                f.values[at]["list"][0]["title"] = json!("Other name");
                f.at("after_home")
            }
            5 => {
                let at = f.at("after_home");
                f.values[at]["data"] = home("888");
                f.values.len()
            }
            _ => {
                let at = f.values.len() - 1;
                f.values[at]["data"]["userId"] = json!("222");
                f.values.len()
            }
        };
        f.truncate(count);
        let (mut p, seen) = server(wire(&f)).await;
        let (store, original, alias) = setup_mode(&mut p, "caller");
        let mut failure = run_metadata(&p, alias, true, PlaylistMetadataUpdateVariant::Default)
            .await
            .unwrap_err();
        assert_eq!(
            failure.details["write_outcome"], "unconfirmed",
            "variant {variant}"
        );
        assert!(!failure.retryable);
        assert!(!failure.message.contains("native-token-fixture"));
        assert!(!failure.details.to_string().contains("native-token-fixture"));
        assert_eq!(read(&store, alias), original);
        if variant == 6 {
            assert!(failure.take_caller_credential_update().is_none());
            assert!(p.take_response_credential().unwrap().is_none());
        } else {
            assert!(failure.take_caller_credential_update().is_some());
        }
        assert_eq!(
            seen.await
                .unwrap()
                .iter()
                .filter(|request| request.starts_with("POST "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn native_playlist_metadata_preserves_new_login_and_discards_post_write_confirmation() {
    let f = flow(false);
    let (mut p, seen, release, requests) = gated(wire(&f)).await;
    let (store, _, alias) = setup_mode(&mut p, "named");
    let task = tokio::spawn(async move {
        run_metadata(&p, alias, false, PlaylistMetadataUpdateVariant::Default).await
    });
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("metadata flow did not reach final identity check")
        .unwrap();
    let replacement = MiguCredential::verified("111".into(), "new-login".into()).unwrap();
    store.put(&stored(alias, &replacement)).unwrap();
    release.send(()).unwrap();
    let failure = task.await.unwrap().unwrap_err();
    assert_eq!(failure.code, ErrorCode::Conflict);
    assert_eq!(failure.details["write_outcome"], "unconfirmed");
    assert!(!failure.retryable);
    assert_eq!(read(&store, alias), replacement);
    requests.abort();
}

#[tokio::test]
async fn native_playlist_metadata_rejects_new_login_before_mutation_dispatch() {
    let mut f = flow(false);
    f.truncate(f.at("validate") + 1);
    let (mut p, seen, release, requests) = gated(wire(&f)).await;
    let (store, _, alias) = setup_mode(&mut p, "named");
    let task = tokio::spawn(async move {
        run_metadata(&p, alias, false, PlaylistMetadataUpdateVariant::Default).await
    });
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("metadata flow did not reach native validation")
        .unwrap();
    let replacement = MiguCredential::verified("111".into(), "new-login".into()).unwrap();
    store.put(&stored(alias, &replacement)).unwrap();
    release.send(()).unwrap();
    let failure = task.await.unwrap().unwrap_err();
    assert_eq!(failure.code, ErrorCode::Conflict);
    // The dispatch marker is set immediately before the native POST; its
    // absence proves this conflict was detected before mutation dispatch.
    assert!(failure.details.get("write_outcome").is_none());
    assert_eq!(read(&store, alias), replacement);
    requests.await.unwrap();
}

#[tokio::test]
async fn native_playlist_metadata_cancellation_discards_pending_caller_update() {
    let f = flow(false);
    let (mut p, seen, release, requests) = gated(wire(&f)).await;
    let (store, original, alias) = setup_mode(&mut p, "caller");
    let p = Arc::new(p);
    let running = p.clone();
    let task = tokio::spawn(async move {
        run_metadata(
            &running,
            alias,
            false,
            PlaylistMetadataUpdateVariant::Default,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("metadata flow did not reach final identity check")
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(p.take_response_credential().unwrap().is_none());
    assert_eq!(read(&store, alias), original);
    drop(release);
    requests.abort();
}
