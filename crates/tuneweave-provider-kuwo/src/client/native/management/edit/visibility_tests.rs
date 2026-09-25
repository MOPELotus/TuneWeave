use super::tests::{directory, metadata};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

pub(crate) fn request(account: Option<&str>) -> PlaylistVisibilityUpdateRequest {
    PlaylistVisibilityUpdateRequest {
        visibility: PlaylistVisibility::Private,
        account: account.map(str::to_owned),
    }
}
pub(crate) fn flow(uid: &str, public: bool, r: &PlaylistVisibilityUpdateRequest) -> Vec<Vec<u8>> {
    vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(public, None)),
        json_response(&metadata(uid, None)),
        json_response(&directory(public, None)),
        json_response(&json!({"errcode":0,"pid":101})),
        json_response(&metadata(uid, None)),
        json_response(&directory(r.visibility == PlaylistVisibility::Public, None)),
    ]
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[tokio::test]
async fn native_visibility_changes_only_public_flag_in_both_directions_and_confirms_same_state() {
    for public in [false, true] {
        for target in [false, true] {
            let mut r = request(None);
            r.visibility = if target {
                PlaylistVisibility::Public
            } else {
                PlaylistVisibility::Private
            };
            let mut f = fixture::setup(flow("42", public, &r)).await;
            let result = f
                .client
                .native_update_playlist_visibility(&credential(), "101", &r)
                .await
                .unwrap();
            assert_eq!(result.action, PlaylistMutationAction::Update);
            assert_eq!(result.extensions["confirmed"], true);
            assert_eq!(result.extensions["atomic"], false);
            let p = result.playlist.as_ref().unwrap();
            assert_eq!(p.extensions["is_public"], target);
            assert_eq!(p.extensions["editable_metadata_verified"], true);
            let original = metadata("42", None);
            assert_eq!(p.name, original["sl_data"]["title"]);
            assert_eq!(p.description, original["sl_data"]["desc"]);
            assert_eq!(p.tags, ["流行", "安静"]);
            assert_eq!(p.track_count, Some(2));
            let seen = fixture::requests(&mut f, 7).await;
            assert_eq!(seen.iter().filter(|v| v.starts_with("POST ")).count(), 1);
            let (head, body) = seen[4].split_once("\r\n\r\n").unwrap();
            assert!(head.contains("op=pl3_editlist"));
            assert!(
                head.to_lowercase()
                    .contains("content-type: application/x-www-form-urlencoded")
            );
            let payload: Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                payload,
                json!({"pid":101,"title":p.name,"intro":p.description,
                "tag":"流行,安静","pic":original["sl_data"]["pic"],"ispub":target})
            );
            for call in &seen {
                assert!(!call.contains("ucheck"));
                assert!(!call.contains("updatelistinfo"));
                assert!(!call.contains("unpublish"));
            }
            let output = serde_json::to_string(&result).unwrap();
            for secret in ["selected-session", "never-export", "do-not-expose"] {
                assert!(!output.contains(secret));
            }
        }
    }
}

#[tokio::test]
async fn native_visibility_rejects_implicit_state_invalid_ids_and_account_alias_before_io() {
    let mut f = fixture::setup(vec![]).await;
    for (id, r) in [
        (
            "101",
            PlaylistVisibilityUpdateRequest::new(PlaylistVisibility::PlatformDefault),
        ),
        ("0", request(None)),
        ("0101", request(None)),
        ("101&uid=7", request(None)),
        ("101", request(Some("personal"))),
    ] {
        assert_eq!(
            f.client
                .native_update_playlist_visibility(&credential(), id, &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    fixture::requests(&mut f, 0).await;
}

#[tokio::test]
async fn native_visibility_requires_owned_ordinary_playlist_and_complete_stable_metadata() {
    let r = request(None);
    for id in ["901", "999"] {
        let mut f = fixture::setup(flow("42", true, &r)[..2].to_vec()).await;
        assert_eq!(
            f.client
                .native_update_playlist_visibility(&credential(), id, &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        fixture::requests(&mut f, 2).await;
    }
    let mut cases = Vec::new();
    for key in ["ispub", "musicnum"] {
        let mut d = directory(true, None);
        d["plist"][0].as_object_mut().unwrap().remove(key);
        cases.push((1, json_response(&d), 3));
    }
    for key in [
        "title", "desc", "tag", "pic", "big_pic", "uid", "total", "igsl",
    ] {
        let mut m = metadata("42", None);
        m["sl_data"].as_object_mut().unwrap().remove(key);
        cases.push((2, json_response(&m), 3));
    }
    cases.push((2, json_response(&metadata("7", None)), 3));
    for (key, value) in [
        ("title", json!("new title")),
        ("ispub", json!(false)),
        ("type", json!("RADIO")),
    ] {
        let mut d = directory(true, None);
        d["plist"][0][key] = value;
        cases.push((3, json_response(&d), 4));
    }
    for (at, response, count) in cases {
        let mut bodies = flow("42", true, &r);
        bodies[at] = response;
        bodies.truncate(count);
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_update_playlist_visibility(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none());
        assert!(!format!("{e:?}").contains("selected-session"));
        assert!(
            fixture::requests(&mut f, count)
                .await
                .iter()
                .all(|s| s.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn native_visibility_unconfirmed_ack_or_changed_readback_never_retries_or_reports_success() {
    let r = request(None);
    let mut cases = vec![
        (4, json_response(&json!({"errcode":603}))),
        (4, json_response(&json!({"errcode":0,"pid":102}))),
        (
            4,
            response(
                302,
                "application/json",
                "Location: https://evil.test/\r\n",
                b"{}",
            ),
        ),
        (6, json_response(&directory(true, None))),
    ];
    for (key, value) in [
        ("title", json!("changed")),
        ("desc", json!("")),
        ("tag", json!("其他,安静")),
        ("tagid", json!("900,400")),
        ("pic", json!("")),
        ("big_pic", json!("")),
        ("total", json!(3)),
        ("uid", json!(7)),
        ("igsl", json!("1")),
        ("playlist_type", json!(1)),
    ] {
        let mut m = metadata("42", None);
        m["sl_data"][key] = value;
        cases.push((5, json_response(&m)));
    }
    for (key, value) in [
        ("ispub", Value::Null),
        ("title", json!("changed")),
        ("info", json!("")),
        ("pic", json!("")),
        ("musicnum", json!(3)),
        ("type", json!("RADIO")),
    ] {
        let mut d = directory(false, None);
        d["plist"][0][key] = value;
        cases.push((6, json_response(&d)));
    }
    for (at, bad) in cases {
        let mut bodies = flow("42", true, &r);
        bodies[at] = bad;
        bodies.truncate(at + 1);
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_update_playlist_visibility(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(e.details["automatic_retry"], false);
        assert!(!e.retryable);
        assert_eq!(
            fixture::requests(&mut f, at + 1)
                .await
                .iter()
                .filter(|s| s.starts_with("POST "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn native_visibility_preserves_explicit_empty_metadata_and_optional_type_fields() {
    let r = request(None);
    let mut bodies = flow("42", true, &r);
    for at in [1, 3, 6] {
        let mut d = directory(at != 6, None);
        d["plist"][0]["info"] = json!("");
        d["plist"][0]["pic"] = json!("");
        bodies[at] = json_response(&d);
    }
    for at in [2, 5] {
        let mut m = metadata("42", None);
        for key in ["desc", "tag", "tagid", "pic", "big_pic"] {
            m["sl_data"][key] = json!("");
        }
        m["sl_data"]["igsl"] = json!("0");
        m["sl_data"]["playlist_type"] = json!(1);
        bodies[at] = json_response(&m);
    }
    let mut f = fixture::setup(bodies).await;
    let p = f
        .client
        .native_update_playlist_visibility(&credential(), "101", &r)
        .await
        .unwrap()
        .playlist
        .unwrap();
    assert!(p.tags.is_empty());
    assert!(p.description.is_empty());
    assert!(p.cover_url.is_none());
    let seen = fixture::requests(&mut f, 7).await;
    let body: Value = serde_json::from_str(seen[4].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        json!({"pid":101,"title":"原名字","intro":"","tag":"","pic":"","ispub":false})
    );
}

#[tokio::test]
async fn native_visibility_rejects_published_contributions_before_any_write() {
    for public in [false, true] {
        for visibility in [PlaylistVisibility::Public, PlaylistVisibility::Private] {
            let r = PlaylistVisibilityUpdateRequest::new(visibility);
            let mut bodies = flow("42", public, &r);
            let mut m = metadata("42", None);
            m["sl_data"]["igsl"] = json!("1");
            bodies[2] = json_response(&m);
            bodies.truncate(3);
            let mut f = fixture::setup(bodies).await;
            let e = f
                .client
                .native_update_playlist_visibility(&credential(), "101", &r)
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::CapabilityNotSupported);
            assert!(e.details.get("write_outcome").is_none());
            let seen = fixture::requests(&mut f, 3).await;
            assert!(seen.iter().all(|v| v.starts_with("GET ")));
        }
    }
}
