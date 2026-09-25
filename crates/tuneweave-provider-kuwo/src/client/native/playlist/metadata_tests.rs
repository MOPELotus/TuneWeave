use super::tests::{detail, directory, flow};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
fn replace_details(bodies: &mut [Vec<u8>], value: &Value) {
    for at in [2, 7] {
        bodies[at] = json_response(value);
    }
}

#[tokio::test]
async fn native_full_metadata_reads_public_private_and_published_without_writes_or_cover_replacement()
 {
    let small = "http://img1.kwcdn.kuwo.cn/star/userpl2015/cover_150.jpg";
    let big = "http://img1.kwcdn.kuwo.cn/star/userpl2015/cover_1260.jpg";
    for public in [false, true] {
        for published in [false, true] {
            for cover in [small, big] {
                let mut bodies = flow();
                let mut m = detail("42", 4);
                m["sl_data"]["pic"] = json!(small);
                m["sl_data"]["big_pic"] = json!(big);
                m["sl_data"]["igsl"] = json!(if published { "1" } else { "0" });
                m["sl_data"]["tag"] = json!("原始 + %20,流行 &amp;");
                replace_details(&mut bodies, &m);
                for at in [1, 8] {
                    let mut d = directory(Some(4));
                    d["plist"][0]["pic"] = json!(cover);
                    d["plist"][0]["ispub"] = json!(public);
                    bodies[at] = json_response(&d);
                }
                let mut f = fixture::setup(bodies).await;
                let p = f
                    .client
                    .native_created_playlist(&credential(), "101")
                    .await
                    .unwrap();
                assert_eq!(p.cover_url.as_deref(), Some(cover));
                assert_eq!(p.tags, ["原始 + %20", "流行 &amp;"]);
                assert_eq!(p.extensions["is_public"], public);
                assert_eq!(p.extensions["editable_metadata_verified"], true);
                assert_eq!(p.extensions["upstream_pages_fetched"], 8);
                let encoded = serde_json::to_string(&p).unwrap();
                for secret in ["selected-session", "never-export", "private-rights"] {
                    assert!(!encoded.contains(secret));
                }
                let seen = fixture::requests(&mut f, 9).await;
                assert!(seen.iter().all(|v| v.starts_with("GET ")));
                assert!(seen.iter().all(|v| !v.contains("ucheck")));
            }
        }
    }
}

#[tokio::test]
async fn native_full_metadata_known_empty_tags_are_preserved_but_unknown_fields_never_fall_back() {
    let mut empty = detail("42", 4);
    empty["sl_data"]["tag"] = json!("");
    empty["sl_data"]["tagid"] = json!("");
    let mut bodies = flow();
    replace_details(&mut bodies, &empty);
    let mut f = fixture::setup(bodies).await;
    let p = f
        .client
        .native_created_playlist(&credential(), "101")
        .await
        .unwrap();
    assert!(p.tags.is_empty());
    assert_eq!(p.extensions["editable_metadata_verified"], true);
    fixture::requests(&mut f, 9).await;
    for at in [2, 7] {
        for key in [
            "title", "desc", "tag", "pic", "big_pic", "uid", "total", "igsl",
        ] {
            for null in [false, true] {
                let mut m = detail("42", 4);
                if null {
                    m["sl_data"][key] = Value::Null;
                } else {
                    m["sl_data"].as_object_mut().unwrap().remove(key);
                }
                let mut bodies = flow();
                bodies[at] = json_response(&m);
                bodies.truncate(at + 1);
                let mut f = fixture::setup(bodies).await;
                let e = f
                    .client
                    .native_created_playlist(&credential(), "101")
                    .await
                    .unwrap_err();
                assert_eq!(e.code, ErrorCode::UpstreamError);
                assert!(e.details.get("write_outcome").is_none());
                fixture::requests(&mut f, at + 1).await;
            }
        }
    }
}

#[tokio::test]
async fn native_full_metadata_any_changed_known_field_between_reads_rejects_unchanged_tracks() {
    for (key, value) in [
        ("title", json!("changed")),
        ("desc", json!("changed")),
        ("tag", json!("流行,新标签")),
        ("tagid", json!("393,500")),
        ("tagid", Value::Null),
        ("pic", json!("https://img4.kuwo.cn/star/albumcover/new.jpg")),
        (
            "big_pic",
            json!("https://img4.kuwo.cn/star/albumcover/new.jpg"),
        ),
        ("uid", json!("7")),
        ("id", json!("102")),
        ("total", json!(5)),
        ("igsl", json!("1")),
        ("playlist_type", json!(1)),
    ] {
        let mut m = detail("42", 4);
        m["sl_data"][key] = value;
        let mut bodies = flow();
        bodies[7] = json_response(&m);
        bodies.truncate(8);
        let mut f = fixture::setup(bodies).await;
        assert_eq!(
            f.client
                .native_created_playlist_tracks(&credential(), "101", &PageRequest::new(1, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "{key}"
        );
        fixture::requests(&mut f, 8).await;
    }
}

#[tokio::test]
async fn native_full_metadata_snapshot_changes_for_tags_and_other_known_details_between_calls() {
    for (key, value, changes_identity) in [
        ("tag", json!("流行,新标签"), true),
        ("tagid", json!("393,500"), true),
        ("tagid", Value::Null, true),
        ("igsl", json!("1"), true),
        ("playlist_type", json!(1), true),
        ("igsl", json!(""), false),
        ("token", json!("ignored-and-never-exported"), false),
    ] {
        let mut second = flow();
        let mut m = detail("42", 4);
        m["sl_data"][key] = value;
        replace_details(&mut second, &m);
        let mut bodies = flow();
        bodies.extend(second);
        let mut f = fixture::setup(bodies).await;
        let a = f
            .client
            .native_created_playlist(&credential(), "101")
            .await
            .unwrap();
        let b = f
            .client
            .native_created_playlist_tracks(&credential(), "101", &PageRequest::new(2, 1))
            .await
            .unwrap();
        assert_eq!(
            a.extensions["source_snapshot_id"] != b.pagination.extensions["source_snapshot_id"],
            changes_identity,
            "{key}"
        );
        assert_eq!(
            b.items.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["22", "22"]
        );
        assert!(!serde_json::to_string(&a).unwrap().contains("never-export"));
        fixture::requests(&mut f, 18).await;
    }
}

#[tokio::test]
async fn native_full_metadata_can_resolve_unknown_directory_count_without_inventing_visibility() {
    let mut bodies = flow();
    for at in [1, 8] {
        let mut d = directory(None);
        d["plist"][0].as_object_mut().unwrap().remove("ispub");
        bodies[at] = json_response(&d);
    }
    let mut f = fixture::setup(bodies).await;
    let p = f
        .client
        .native_created_playlist(&credential(), "101")
        .await
        .unwrap();
    assert_eq!(p.track_count, Some(4));
    assert_eq!(p.tags, ["流行", "安静"]);
    assert_eq!(p.extensions.get("is_public"), Some(&Value::Null));
    fixture::requests(&mut f, 9).await;
    let mut bodies = flow();
    bodies[1] = json_response(&directory(None));
    bodies[2] = json_response(&detail("42", MAX_TRACKS as u64 + 1));
    bodies.truncate(3);
    let mut f = fixture::setup(bodies).await;
    assert!(
        f.client
            .native_created_playlist(&credential(), "101")
            .await
            .is_err()
    );
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_full_metadata_transport_limits_identity_and_reflections_apply_at_both_reads() {
    for at in [2, 7] {
        let mut reflected = detail("42", 4);
        reflected["sl_data"]["tag"] = json!("流行,selected-session");
        for bad in [
            response(200, "text/html", "", b"{}"),
            response(
                302,
                "application/json",
                "Location: https://evil.test/\r\n",
                b"{}",
            ),
            response(403, "application/json", "", b"{}"),
            response(
                200,
                "application/json",
                "",
                &vec![b' '; library::MAX_RESPONSE + 1],
            ),
            json_response(&detail("7", 4)),
            json_response(&reflected),
        ] {
            let mut bodies = flow();
            bodies[at] = bad;
            bodies.truncate(at + 1);
            let mut f = fixture::setup(bodies).await;
            let e = f
                .client
                .native_created_playlist(&credential(), "101")
                .await
                .unwrap_err();
            assert!(!format!("{e:?}").contains("selected-session"));
            fixture::requests(&mut f, at + 1).await;
        }
    }
}
