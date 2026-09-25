use super::*;
use crate::client::playlist_order::ORDER_PATH;
use crate::provider::account_media::tests::setup as setup_mode;
use std::collections::BTreeMap;
use tuneweave_core::PlaylistTrackOrderRequest;

const RAW_NAME: &str = "  测试 A & + # / 100%  ";
const CHANGED_COVER: &str = "https://d.musicapp.migu.cn/data/oss/changed-first-cover.png";

mod duplicates;
mod occurrences;

fn flow(before: &[u32], after: &[Vec<u32>]) -> Flow {
    let mut f = Flow::new();
    f.profile();
    f.data("before_home", home("999"));
    let created: Vec<_> = std::iter::once(77).chain(1..=20).collect();
    f.library("before_library", &created, false);
    f.snapshot(false, before);
    if !after.is_empty() {
        f.push("native_profile", json!({"code":"000000","data":{"userId":"111","nickName":"Account","usessionId":"native-session-fixture"}}));
        f.pair(
            "exchange",
            json!({"code":"000000","data":"native-token-fixture"}),
        );
        f.push(
            "validate",
            json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}}),
        );
    }
    for ids in after {
        f.push("move", json!({"code":"000000"}));
        let start = f.values.len();
        f.snapshot(true, ids);
        if before.first() != ids.first() {
            for (label, value) in f.labels.iter().zip(&mut f.values).skip(start) {
                if matches!(*label, "after_metadata" | "after_metadata_final") {
                    value["data"]["originalImgUrl"] = json!(CHANGED_COVER);
                }
            }
        }
    }
    f.library("after_library", &created, false);
    f.data("after_home", home("999"));
    for (label, value) in f.labels.iter().zip(&mut f.values) {
        if matches!(*label, "before_tracks" | "after_tracks") {
            for song in value["data"]["songList"].as_array_mut().unwrap() {
                if song["contentId"] == "51" {
                    song["songName"] = json!(RAW_NAME);
                    song["singerList"] =
                        json!([{"id":"123","name":"Artist A"},{"id":"456","name":"Artist B"}]);
                }
            }
        }
    }
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

fn request(ids: &[u32], alias: &str) -> PlaylistTrackOrderRequest {
    PlaylistTrackOrderRequest {
        track_refs: ids
            .iter()
            .map(|id| ResourceRef::new(Platform::Migu, id.to_string()).unwrap())
            .collect(),
        account: Some(alias.into()),
    }
}

fn orders() -> (Vec<u32>, Vec<Vec<u32>>) {
    let original = (1..=51).collect::<Vec<_>>();
    let mut first = original.clone();
    let last = first.pop().unwrap();
    first.insert(0, last);
    let mut second = first.clone();
    second.swap(1, 2);
    (original, vec![first, second])
}

#[tokio::test]
async fn native_playlist_order_minimum_plan_moves_across_pages_in_both_directions_once() {
    let original = (1..=51).collect::<Vec<_>>();
    for forward in [true, false] {
        let mut desired = original.clone();
        if forward {
            desired.rotate_left(1);
        } else {
            desired.rotate_right(1);
        }
        for mode in ["default", "named", "caller"] {
            let f = flow(&original, std::slice::from_ref(&desired));
            let (mut provider, seen) = server(wire(&f)).await;
            let (store, initial, alias) = setup_mode(&mut provider, mode);
            let request = request(&desired, alias);
            let result = provider
                .reorder_playlist_tracks("77", &request)
                .await
                .unwrap();
            assert_eq!(result.track_refs, request.track_refs);
            assert_eq!(result.extensions["moves_dispatched"], 1);
            assert_eq!(result.extensions["atomic"], true);
            assert_eq!(result.extensions["cover_changed"], true);
            let confirmed = result.extensions["confirmed_moves"].as_array().unwrap();
            assert_eq!(confirmed.len(), 1);
            assert_eq!(confirmed[0]["old_position"], if forward { 1 } else { 51 });
            assert_eq!(confirmed[0]["new_position"], if forward { 51 } else { 1 });
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if mode == "caller" {
                assert_eq!(read(&store, alias), initial);
                let update = provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    format!("p{}", f.values.len() - 1)
                );
                assert!(!update.secret().contains("native-token-fixture"));
                assert!(provider.take_response_credential().unwrap().is_none());
            } else {
                assert_eq!(
                    read(&store, alias).token(),
                    format!("p{}", f.values.len() - 1)
                );
            }
            let requests = seen.await.unwrap();
            assert_eq!(requests.len(), f.values.len());
            let moves = requests
                .iter()
                .filter(|value| value.contains(ORDER_PATH))
                .collect::<Vec<_>>();
            assert_eq!(moves.len(), 1);
            let query = moves[0]
                .lines()
                .next()
                .unwrap()
                .split_once('?')
                .unwrap()
                .1
                .strip_suffix(" HTTP/1.1")
                .unwrap();
            let fields = url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect::<BTreeMap<_, _>>();
            let id = if forward { "1" } else { "51" };
            assert_eq!(fields["contentId"], id);
            assert_eq!(fields["songId"], format!("s{id}"));
            assert_eq!(fields["oldPostion"], id);
            assert_eq!(fields["newPosition"], if forward { "51" } else { "1" });
            assert!(moves[0].contains("token: native-token-fixture\r\n"));
            assert!(moves[0].contains("signversion: V005\r\n"));
            assert!(!moves[0].contains("pacmtoken:"));
        }
    }
}

#[tokio::test]
async fn native_playlist_order_minimum_plan_forward_failure_never_retries_or_claims_success() {
    let original = (1..=51).collect::<Vec<_>>();
    let mut desired = original.clone();
    desired.rotate_left(1);
    for refused in [true, false] {
        let mut f = flow(&original, std::slice::from_ref(&desired));
        if refused {
            let at = f.at("move");
            f.values[at] = json!({"code":"200013","info":"native-token-fixture"});
            f.truncate(at + 1);
        } else {
            let at = f.at("after_tracks");
            f.values[at]["data"]["songList"]
                .as_array_mut()
                .unwrap()
                .swap(0, 1);
            f.truncate(f.at("after_library"));
        }
        let (mut provider, seen) = server(wire(&f)).await;
        setup_mode(&mut provider, "named");
        let failure = provider
            .reorder_playlist_tracks("77", &request(&desired, "personal"))
            .await
            .unwrap_err();
        assert_eq!(
            failure.code,
            if refused {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert_eq!(failure.details["moves_dispatched"], 1);
        assert_eq!(failure.details["confirmed_moves"], json!([]));
        assert!(!failure.retryable);
        assert!(!format!("{failure:?}").contains("native-token-fixture"));
        assert_eq!(
            seen.await
                .unwrap()
                .iter()
                .filter(|value| value.contains(ORDER_PATH))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn native_playlist_order_minimum_plan_forward_preserves_generation_after_dispatch() {
    let original = (1..=51).collect::<Vec<_>>();
    let mut desired = original.clone();
    desired.rotate_left(1);
    for boundary in ["move", "after_metadata"] {
        let mut f = flow(&original, std::slice::from_ref(&desired));
        f.truncate(f.at(boundary) + 1);
        let (mut provider, seen, release, server_task) = gated(wire(&f)).await;
        let (store, _, alias) = setup_mode(&mut provider, "named");
        let provider = Arc::new(provider);
        let running = provider.clone();
        let request = request(&desired, alias);
        let task =
            tokio::spawn(async move { running.reorder_playlist_tracks("77", &request).await });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .unwrap()
            .unwrap();
        let replacement =
            MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
        store.put(&stored(alias, &replacement)).unwrap();
        release.send(()).unwrap();
        let failure = task.await.unwrap().unwrap_err();
        assert_eq!(failure.code, ErrorCode::Conflict);
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert_eq!(failure.details["moves_dispatched"], 1);
        assert_eq!(failure.details["confirmed_moves"], json!([]));
        assert!(!failure.retryable);
        assert_eq!(read(&store, alias), replacement);
        server_task.await.unwrap();
    }
}

#[tokio::test]
async fn native_playlist_order_moves_use_original_fields_and_complete_selected_account_readback() {
    let (original, stages) = orders();
    for mode in ["default", "named", "caller"] {
        let f = flow(&original, &stages);
        let (mut provider, seen) = server(wire(&f)).await;
        let (store, initial, alias) = setup_mode(&mut provider, mode);
        let request = request(stages.last().unwrap(), alias);
        let result = provider
            .reorder_playlist_tracks("77", &request)
            .await
            .unwrap();
        assert_eq!(result.track_refs, request.track_refs);
        assert!(
            result
                .snapshot_id
                .as_ref()
                .unwrap()
                .starts_with("migu_account_playlist_v1_")
        );
        assert_eq!(result.extensions["moves_dispatched"], 2);
        assert_eq!(result.extensions["confirmed_moves"][0]["old_position"], 51);
        assert_eq!(result.extensions["confirmed_moves"][0]["new_position"], 1);
        assert_eq!(result.extensions["confirmed_moves"][1]["old_position"], 3);
        assert_eq!(result.extensions["confirmed_moves"][1]["new_position"], 2);
        assert_eq!(result.extensions["atomic"], false);
        assert_eq!(result.extensions["cover_changed"], true);
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), initial);
            let credential = provider.take_response_credential().unwrap().unwrap();
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
        let moves = requests
            .iter()
            .filter(|r| r.starts_with(&format!("GET {ORDER_PATH}?")))
            .collect::<Vec<_>>();
        assert_eq!(moves.len(), 2);
        let query = moves[0]
            .lines()
            .next()
            .unwrap()
            .split_once('?')
            .unwrap()
            .1
            .strip_suffix(" HTTP/1.1")
            .unwrap();
        let fields = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(fields.len(), 7);
        assert_eq!(fields["musicList"], "77");
        assert_eq!(fields["contentId"], "51");
        assert_eq!(fields["songId"], "s51");
        assert_eq!(fields["songName"], RAW_NAME);
        assert_eq!(fields["singer"], "Artist A|Artist B");
        assert_eq!(fields["oldPostion"], "51");
        assert_eq!(fields["newPosition"], "1");
        assert!(query.contains("%20"));
        assert!(!query.contains('+'));
        assert!(query.find("contentId=").unwrap() < query.find("musicList=").unwrap());
        for movement in moves {
            assert!(movement.contains("token: native-token-fixture\r\n"));
            assert!(movement.contains("signversion: V005\r\n"));
            assert!(!movement.contains("pacmtoken:"));
            assert!(!movement.contains("native-session-fixture"));
        }
        assert!(!requests.iter().any(|r| r.starts_with("POST ")));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("native-token-fixture")
        );
    }
}

#[tokio::test]
async fn native_playlist_order_noop_still_confirms_library_without_native_write() {
    let ids = vec![1, 2, 3];
    let f = flow(&ids, &[]);
    let (mut provider, seen) = server(wire(&f)).await;
    setup_mode(&mut provider, "named");
    let result = provider
        .reorder_playlist_tracks("77", &request(&ids, "personal"))
        .await
        .unwrap();
    assert_eq!(result.extensions["moves_dispatched"], 0);
    assert_eq!(result.extensions["confirmed_moves"], json!([]));
    assert!(
        !seen
            .await
            .unwrap()
            .iter()
            .any(|r| r.contains(ORDER_PATH) || r.contains("token-validate"))
    );
}

#[tokio::test]
async fn native_playlist_order_rejects_incomplete_ambiguous_or_excessive_orders_before_dispatch() {
    let (original, stages) = orders();
    for variant in 0..7 {
        let mut f = flow(&original, &stages[..1]);
        let mut desired = stages[0].clone();
        let expected_code = match variant {
            0 => {
                desired.pop();
                ErrorCode::InvalidRequest
            }
            1 => {
                desired = original.iter().rev().copied().collect();
                ErrorCode::CapabilityNotSupported
            }
            2 | 3 => {
                for (label, value) in f.labels.iter().zip(&mut f.values) {
                    if *label == "before_tracks" {
                        for song in value["data"]["songList"].as_array_mut().unwrap() {
                            if song["contentId"] == "2" {
                                if variant == 2 {
                                    song["songId"] = json!("s1");
                                } else {
                                    song.as_object_mut().unwrap().remove("songId");
                                }
                            }
                        }
                    }
                }
                ErrorCode::CapabilityNotSupported
            }
            4 => {
                for (label, value) in f.labels.iter().zip(&mut f.values) {
                    if matches!(
                        *label,
                        "before_metadata" | "before_metadata_final" | "before_tracks"
                    ) {
                        value["data"]["ownerId"] = json!("222");
                    }
                }
                ErrorCode::PermissionDenied
            }
            5 => {
                let at = f.at("before_metadata_final");
                f.values[at]["data"]["originalImgUrl"] = json!(CHANGED_COVER);
                ErrorCode::UpstreamError
            }
            _ => {
                for (label, value) in f.labels.iter().zip(&mut f.values) {
                    if *label == "before_tracks" {
                        for song in value["data"]["songList"].as_array_mut().unwrap() {
                            if song["contentId"] == "2" {
                                song["contentId"] = json!("1");
                            }
                        }
                    }
                }
                ErrorCode::CapabilityNotSupported
            }
        };
        f.truncate(f.at("native_profile"));
        let (mut provider, seen) = server(wire(&f)).await;
        setup_mode(&mut provider, "named");
        let failure = provider
            .reorder_playlist_tracks("77", &request(&desired, "personal"))
            .await
            .unwrap_err();
        assert_eq!(failure.code, expected_code, "variant {variant}");
        assert!(failure.details.get("write_outcome").is_none());
        assert!(!seen.await.unwrap().iter().any(|r| r.contains(ORDER_PATH)));
    }
}

#[tokio::test]
async fn native_playlist_order_reports_partial_moves_and_rejects_changed_or_stale_readback() {
    let (original, stages) = orders();
    for variant in 0..6 {
        let mut f = flow(&original, &stages);
        let second_move = f
            .labels
            .iter()
            .enumerate()
            .filter(|(_, label)| **label == "move")
            .nth(1)
            .unwrap()
            .0;
        if variant == 0 {
            f.values[second_move] = json!({"code":"200013","info":"native-token-fixture"});
            f.truncate(second_move + 1);
        } else {
            if variant == 1 {
                let at = f.at("after_tracks");
                f.values[at]["data"]["songList"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 1);
            } else if variant == 2 {
                for (label, value) in f.labels.iter().zip(&mut f.values).take(second_move) {
                    if matches!(*label, "after_metadata" | "after_metadata_final") {
                        value["data"]["summary"] = json!("Unrelated edit");
                    }
                }
            } else if variant == 3 {
                let at = f.at("after_tracks");
                f.values[at]["data"]["songList"][0]["songId"] = json!("unexpected-native-id");
            } else if variant == 4 {
                let at = f.at("after_metadata_final");
                f.values[at]["data"]["originalImgUrl"] =
                    json!("https://d.musicapp.migu.cn/data/oss/other-cover.png");
            } else {
                for (label, value) in f.labels.iter().zip(&mut f.values).take(second_move) {
                    if matches!(*label, "after_metadata" | "after_metadata_final") {
                        value["data"]
                            .as_object_mut()
                            .unwrap()
                            .remove("originalImgUrl");
                    }
                }
            }
            f.truncate(second_move);
        }
        let (mut provider, seen) = server(wire(&f)).await;
        setup_mode(&mut provider, "caller");
        let failure = provider
            .reorder_playlist_tracks("77", &request(&stages[1], "default"))
            .await
            .unwrap_err();
        assert_eq!(
            failure.details["write_outcome"], "unconfirmed",
            "variant {variant}"
        );
        assert_eq!(
            failure.details["moves_dispatched"],
            if variant == 0 { 2 } else { 1 }
        );
        assert_eq!(
            failure.details["confirmed_moves"].as_array().unwrap().len(),
            usize::from(variant == 0)
        );
        assert!(!failure.retryable);
        assert!(!failure.details.to_string().contains("native-token-fixture"));
        assert_eq!(
            seen.await
                .unwrap()
                .iter()
                .filter(|r| r.contains(ORDER_PATH))
                .count(),
            if variant == 0 { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn native_playlist_order_preserves_cover_when_first_song_is_unchanged() {
    let original = vec![1, 2, 3];
    let changed = vec![1, 3, 2];
    let mut f = flow(&original, std::slice::from_ref(&changed));
    for (label, value) in f.labels.iter().zip(&mut f.values) {
        if matches!(*label, "after_metadata" | "after_metadata_final") {
            value["data"]["originalImgUrl"] = json!(CHANGED_COVER);
        }
    }
    f.truncate(f.at("after_library"));
    let (mut provider, seen) = server(wire(&f)).await;
    setup_mode(&mut provider, "named");
    let failure = provider
        .reorder_playlist_tracks("77", &request(&changed, "personal"))
        .await
        .unwrap_err();
    assert_eq!(failure.details["write_outcome"], "unconfirmed");
    assert_eq!(failure.details["confirmed_moves"], json!([]));
    assert!(!failure.retryable);
    assert_eq!(
        seen.await
            .unwrap()
            .iter()
            .filter(|r| r.contains(ORDER_PATH))
            .count(),
        1
    );
}

#[tokio::test]
async fn native_playlist_order_rejects_replacement_login_before_dispatch_and_clears_cancelled_caller_state()
 {
    let (original, stages) = orders();
    for cancel in [false, true] {
        let mut f = flow(&original, &stages[..1]);
        f.truncate(f.at("validate") + 1);
        let (mut provider, seen, release, server_task) = gated(wire(&f)).await;
        let (store, initial, alias) =
            setup_mode(&mut provider, if cancel { "caller" } else { "named" });
        let provider = Arc::new(provider);
        let running = provider.clone();
        let request = request(&stages[0], alias);
        let task =
            tokio::spawn(async move { running.reorder_playlist_tracks("77", &request).await });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .expect("order flow did not reach native validation")
            .unwrap();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(read(&store, alias), initial);
            drop(release);
            server_task.abort();
        } else {
            let replacement =
                MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
            store.put(&stored(alias, &replacement)).unwrap();
            release.send(()).unwrap();
            let failure = task.await.unwrap().unwrap_err();
            assert_eq!(failure.code, ErrorCode::Conflict);
            assert!(failure.details.get("write_outcome").is_none());
            assert_eq!(read(&store, alias), replacement);
            server_task.await.unwrap();
        }
    }
}
