use super::super::tests::{page, request, row, store_account};
use super::*;
use crate::provider::session::tests::{exchange, profile, read, reply, server};
use serde_json::Value;

fn favorite() -> Value {
    let mut p = row(37, 0);
    p["name"] = json!("Renamed likes");
    p["is_def"] = json!(2);
    p
}
fn tracks() -> String {
    reply(
        json!({"userid":111,"listid":37,"type":0,"list_ver":3,"count":2,
    "info":[{"fileid":81,"mixsongid":900,"name":"Song","sort":1},{"fileid":95,"mixsongid":900,"name":"Song","sort":0}]}),
    )
}

#[tokio::test]
async fn favorites_use_unique_created_type_marker_and_preserve_duplicates_in_every_entry_point() {
    for method in [
        "metadata",
        "tracks",
        "user_metadata",
        "user_tracks",
        "source",
        "source_items",
    ] {
        let mut ordinary = row(2, 0);
        ordinary["name"] = json!("我喜欢");
        ordinary["is_def"] = json!(0);
        let mut default = row(1, 0);
        default["is_def"] = json!(1);
        let mut collected = row(99, 1);
        collected["is_def"] = json!(2);
        let lists = page(vec![ordinary, default, collected, favorite()]);
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            lists.clone().into(),
            tracks().into(),
            lists.into(),
        ])
        .await;
        store_account(&mut f.provider);
        let ext = match method {
            "metadata" => {
                f.provider
                    .favorite_playlist(Some("A"))
                    .await
                    .unwrap()
                    .extensions
            }
            "user_metadata" => {
                f.provider
                    .user_favorite_playlist("111", Some("A"))
                    .await
                    .unwrap()
                    .extensions
            }
            "source" => {
                f.provider
                    .playlist_source("111", "favorite_tracks", Some("A"))
                    .await
                    .unwrap()
                    .extensions
            }
            "source_items" => {
                f.provider
                    .playlist_source_items("111", "favorite_tracks", &request("A", 10, 0))
                    .await
                    .unwrap()
                    .pagination
                    .extensions
            }
            _ => {
                let p = if method == "tracks" {
                    f.provider.favorite_tracks(&request("A", 10, 0)).await
                } else {
                    f.provider
                        .user_favorite_tracks("111", &request("A", 10, 0))
                        .await
                }
                .unwrap();
                assert_eq!(p.items.len(), 2);
                assert_eq!(p.items[0].id, p.items[1].id);
                assert_eq!(p.items[0].extensions["file_id"], 95);
                p.pagination.extensions
            }
        };
        assert_eq!(ext["source_type"], "favorite_tracks");
        assert_eq!(ext["favorite_kind"], "kugou");
        assert!(ext["source_snapshot_id"].is_string());
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn favorites_missing_duplicate_or_changed_markers_fail_without_name_or_id_fallback() {
    for case in ["missing", "duplicate", "changed", "new_duplicate"] {
        let mut extra = row(38, 0);
        extra["is_def"] = json!(2);
        let before = match case {
            "missing" => vec![row(2, 0)],
            "duplicate" => vec![favorite(), extra.clone()],
            _ => vec![favorite()],
        };
        let mut frames = vec![
            exchange("111", "next").into(),
            profile("111").into(),
            page(before).into(),
        ];
        if matches!(case, "changed" | "new_duplicate") {
            frames.push(tracks().into());
            frames.push(
                page(if case == "changed" {
                    vec![extra]
                } else {
                    vec![favorite(), extra]
                })
                .into(),
            );
        }
        let n = frames.len();
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        assert_eq!(
            f.provider
                .favorite_playlist(Some("A"))
                .await
                .unwrap_err()
                .code,
            if case == "missing" {
                ErrorCode::ResourceNotFound
            } else {
                ErrorCode::Conflict
            }
        );
        assert_eq!(f.requests.await.unwrap().len(), n);
    }
}

#[tokio::test]
async fn caller_favorites_return_rotation_without_reading_or_writing_server_aliases() {
    let mut f = server(vec![
        exchange("111", "caller-next").into(),
        profile("111").into(),
        page(vec![favorite()]).into(),
        tracks().into(),
        page(vec![favorite()]).into(),
    ])
    .await;
    let store = store_account(&mut f.provider);
    let before = read(&store, "A");
    let caller = f.provider.caller_scope(&before.caller().unwrap()).unwrap();
    assert_eq!(
        caller.favorite_playlist(None).await.unwrap().id,
        "cloudlist:111:0:37"
    );
    assert_eq!(
        KugouCredential::parse_caller(&caller.take_response_credential().unwrap().unwrap())
            .unwrap()
            .native()
            .session
            .token,
        "caller-next"
    );
    assert_eq!(read(&store, "A"), before);
    f.requests.await.unwrap();
}

#[tokio::test]
async fn favorite_user_identity_and_pagination_are_validated_before_network() {
    let mut f = server(vec![]).await;
    store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .user_favorite_playlist("222", Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert_eq!(
            f.provider
                .favorite_tracks(&request("A", limit, offset))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(f.requests.await.unwrap().is_empty());
}
