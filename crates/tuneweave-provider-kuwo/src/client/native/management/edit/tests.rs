use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

const SMALL: &str = "http://img1.kwcdn.kuwo.cn/star/userpl2015/55/15/cover_150.jpg";
const BIG: &str = "http://img1.kwcdn.kuwo.cn/star/userpl2015/55/15/cover_1260.jpg";
pub(crate) fn request(account: Option<&str>) -> PlaylistUpdateRequest {
    PlaylistUpdateRequest {
        name: Some("新名字 + %20 & \"原文\"".into()),
        account: account.map(str::to_owned),
        ..Default::default()
    }
}
pub(crate) fn metadata(uid: &str, r: Option<&PlaylistUpdateRequest>) -> Value {
    let mut value = json!({"sl_data":{"uid":uid,"title":"原名字","desc":"第一行\n第二行","tag":"流行,安静","tagid":"393,400","pic":SMALL,"big_pic":BIG,"total":"2","igsl":"0","token":"never-export","uname":"do-not-expose"}});
    if let Some(r) = r {
        if let Some(name) = &r.name {
            value["sl_data"]["title"] = json!(name);
        }
        if let Some(desc) = &r.description {
            value["sl_data"]["desc"] = json!(desc);
        }
        if let Some(tags) = &r.tags {
            value["sl_data"]["tag"] = json!(tags.join(","));
            value["sl_data"]["tagid"] = json!(if tags.is_empty() { "" } else { "900" });
        }
    }
    value
}
pub(crate) fn directory(public: bool, r: Option<&PlaylistUpdateRequest>) -> Value {
    let m = metadata("42", r);
    json!({"errcode":0,"plist":[{"type":"GENERAL","id":101,"title":m["sl_data"]["title"],"info":m["sl_data"]["desc"],"pic":SMALL,"ispub":public,"musicnum":2},
        {"type":"MYFAVORITE","id":901,"musicnum":0,"ispub":false}]})
}
pub(crate) fn flow(uid: &str, public: bool, r: &PlaylistUpdateRequest) -> Vec<Vec<u8>> {
    vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(public, None)),
        json_response(&metadata(uid, None)),
        json_response(&directory(public, None)),
        json_response(&json!({"errcode":0,"pid":101})),
        json_response(&metadata(uid, Some(r))),
        json_response(&directory(public, Some(r))),
    ]
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[tokio::test]
async fn native_edit_updates_only_requested_fields_and_preserves_original_cover_and_visibility() {
    for public in [false, true] {
        for field in [
            "name",
            "description",
            "tags",
            "clear-description",
            "clear-tags",
            "all",
        ] {
            let mut r = PlaylistUpdateRequest::new();
            match field {
                "name" => r.name = request(None).name,
                "description" => r.description = Some("更新\n保留 + %20 & <字面>".into()),
                "tags" => r.tags = Some(vec!["新标签".into()]),
                "clear-description" => r.description = Some(String::new()),
                "clear-tags" => r.tags = Some(vec![]),
                _ => {
                    r.name = request(None).name;
                    r.description = Some("新简介".into());
                    r.tags = Some(vec!["新标签".into()]);
                }
            }
            let mut f = fixture::setup(flow("42", public, &r)).await;
            let result = f
                .client
                .native_update_playlist(&credential(), "101", &r)
                .await
                .unwrap();
            assert_eq!(result.action, PlaylistMutationAction::Update);
            assert_eq!(result.extensions["confirmed"], true);
            assert_eq!(result.extensions["atomic"], false);
            let p = result.playlist.as_ref().unwrap();
            assert_eq!(p.cover_url.as_deref(), Some(SMALL));
            assert_eq!(p.extensions["is_public"], public);
            assert_eq!(p.name, r.name.as_deref().unwrap_or("原名字"));
            assert_eq!(
                p.description,
                r.description.as_deref().unwrap_or("第一行\n第二行")
            );
            assert_eq!(
                p.tags,
                r.tags
                    .clone()
                    .unwrap_or_else(|| vec!["流行".into(), "安静".into()])
            );
            let encoded = serde_json::to_string(&result).unwrap();
            for secret in ["selected-session", "never-export", "do-not-expose"] {
                assert!(!encoded.contains(secret));
            }
            let seen = fixture::requests(&mut f, 7).await;
            assert_eq!(seen.iter().filter(|v| v.starts_with("POST ")).count(), 1);
            for index in [2, 5] {
                let url = Url::parse(&format!(
                    "https://fixture.test{}",
                    seen[index].split_whitespace().nth(1).unwrap()
                ))
                .unwrap();
                assert_eq!(url.path(), METADATA_PATH);
                let pairs = url.query_pairs().collect::<Vec<_>>();
                let map = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
                assert_eq!(map.len(), pairs.len());
                for (k, v) in [
                    ("type", "get_songlist_info2"),
                    ("id", "101"),
                    ("uid", "42"),
                    ("loginUid", "42"),
                    ("loginSid", "selected-session"),
                    ("apiv", "3"),
                    ("aapiver", "1"),
                    ("pos", "0"),
                    ("newuigroup", "0"),
                ] {
                    assert_eq!(map[k], v);
                }
                assert!(seen[index].contains("loginUid=42,loginSid=selected-session,"));
                assert!(!seen[index].to_lowercase().contains("\r\ncookie:"));
            }
            let (head, body) = seen[4].split_once("\r\n\r\n").unwrap();
            assert!(head.contains("op=pl3_editlist"));
            let payload: Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                payload,
                json!({"pid":101,"title":p.name,"intro":p.description,"tag":p.tags.join(","),"pic":SMALL,"ispub":public})
            );
            assert!(!body.contains("uname"));
            assert!(!body.contains(BIG));
        }
    }
}

#[tokio::test]
async fn native_edit_validates_local_inputs_before_network() {
    let mut f = fixture::setup(vec![]).await;
    let mut values = vec![PlaylistUpdateRequest::new()];
    for name in ["", "  ", "bad\n", &"x".repeat(1025)] {
        let mut r = request(None);
        r.name = Some(name.into());
        values.push(r);
    }
    for description in ["\u{0}".into(), "x".repeat(16385)] {
        let mut r = request(None);
        r.description = Some(description);
        values.push(r);
    }
    for tags in [
        vec![String::new()],
        vec!["a,b".into()],
        vec!["x".into(), "x".into()],
        vec!["x".repeat(257)],
        vec!["\n".into()],
        (0..129).map(|n| n.to_string()).collect(),
        (0..20).map(|n| format!("{n}{}", "x".repeat(230))).collect(),
    ] {
        let mut r = request(None);
        r.tags = Some(tags);
        values.push(r);
    }
    for r in values {
        assert_eq!(
            f.client
                .native_update_playlist(&credential(), "101", &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for variant in [
        PlaylistMetadataUpdateVariant::Batch,
        PlaylistMetadataUpdateVariant::Individual,
    ] {
        let mut r = request(None);
        r.variant = variant;
        assert_eq!(
            f.client
                .native_update_playlist(&credential(), "101", &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }
    for id in ["0", "0101", "101&uid=7"] {
        assert!(
            f.client
                .native_update_playlist(&credential(), id, &request(None))
                .await
                .is_err()
        );
    }
    assert!(
        f.client
            .native_update_playlist(&credential(), "101", &request(Some("personal")))
            .await
            .is_err()
    );
    fixture::requests(&mut f, 0).await;
}

#[tokio::test]
async fn native_edit_missing_or_ambiguous_metadata_never_becomes_empty_write_values() {
    let mut invalids = Vec::new();
    for key in [
        "title", "desc", "tag", "pic", "big_pic", "uid", "total", "igsl",
    ] {
        let mut value = metadata("42", None);
        value["sl_data"].as_object_mut().unwrap().remove(key);
        invalids.push(value);
        let mut value = metadata("42", None);
        value["sl_data"][key] = Value::Null;
        invalids.push(value);
    }
    for (key, bad) in [
        ("uid", json!("7")),
        ("id", json!(102)),
        ("title", json!("")),
        ("title", json!("selected-session")),
        ("desc", json!("different")),
        ("total", json!(3)),
        ("tag", json!("a,,b")),
        ("tagid", json!("393")),
        ("pic", json!("https://img1.kwcdn.kuwo.cn.evil.test/x")),
        (
            "big_pic",
            json!("https://img1.kwcdn.kuwo.cn/x?sid=selected-session"),
        ),
        ("igsl", json!("unknown")),
    ] {
        let mut value = metadata("42", None);
        value["sl_data"][key] = bad;
        invalids.push(value);
    }
    for value in invalids {
        let r = request(None);
        let mut replies = flow("42", false, &r);
        replies[2] = json_response(&value);
        replies.truncate(3);
        let mut f = fixture::setup(replies).await;
        let e = f
            .client
            .native_update_playlist(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none());
        assert!(!format!("{e:?}").contains("selected-session"));
        fixture::requests(&mut f, 3).await;
    }
}

#[tokio::test]
async fn native_edit_explicit_empty_cover_is_preserved_and_does_not_use_a_default_picture() {
    let r = request(None);
    let mut bodies = flow("42", false, &r);
    for (index, after) in [(1, false), (3, false), (6, true)] {
        let mut d = directory(false, after.then_some(&r));
        d["plist"][0]["pic"] = json!("");
        bodies[index] = json_response(&d);
    }
    for (index, after) in [(2, false), (5, true)] {
        let mut m = metadata("42", after.then_some(&r));
        m["sl_data"]["pic"] = json!("");
        m["sl_data"]["big_pic"] = json!("");
        m["sl_data"]["igsl"] = json!("");
        bodies[index] = json_response(&m);
    }
    let mut f = fixture::setup(bodies).await;
    assert!(
        f.client
            .native_update_playlist(&credential(), "101", &r)
            .await
            .unwrap()
            .playlist
            .unwrap()
            .cover_url
            .is_none()
    );
    let seen = fixture::requests(&mut f, 7).await;
    let payload: Value = serde_json::from_str(seen[4].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(payload["pic"], "");
}

#[tokio::test]
async fn native_edit_metadata_transport_rejects_bad_mime_oversize_and_redirect_without_writes() {
    for bad in [
        response(200, "text/html", "", b"{}"),
        response(
            200,
            "application/json",
            "",
            &vec![b' '; library::MAX_RESPONSE + 1],
        ),
        response(
            302,
            "application/json",
            "Location: https://evil.test/\r\n",
            b"{}",
        ),
        response(403, "application/json", "", b"{}"),
    ] {
        let r = request(None);
        let mut bodies = flow("42", true, &r);
        bodies[2] = bad;
        bodies.truncate(3);
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_update_playlist(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none());
        fixture::requests(&mut f, 3).await;
    }
}

#[tokio::test]
async fn native_edit_rejects_nonordinary_unknown_visibility_and_prewrite_changes() {
    for id in ["901", "999"] {
        let mut f = fixture::setup(flow("42", false, &request(None))[..2].to_vec()).await;
        assert_eq!(
            f.client
                .native_update_playlist(&credential(), id, &request(None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        fixture::requests(&mut f, 2).await;
    }
    for (boundary, key, value) in [
        (1, "ispub", Value::Null),
        (1, "musicnum", Value::Null),
        (3, "title", json!("concurrent change")),
        (3, "ispub", json!(true)),
        (3, "pic", json!(BIG)),
        (3, "type", json!("RADIO")),
    ] {
        let r = request(None);
        let mut bodies = flow("42", false, &r);
        let mut dir = directory(false, None);
        dir["plist"][0][key] = value;
        bodies[boundary] = json_response(&dir);
        bodies.truncate(if boundary == 1 { 3 } else { 4 });
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_update_playlist(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none());
        fixture::requests(&mut f, if boundary == 1 { 3 } else { 4 }).await;
    }
}

#[tokio::test]
async fn native_edit_write_ack_and_all_readback_fields_must_be_confirmed_without_retry() {
    let r = request(None);
    for (at, bad) in [
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
        (5, json_response(&metadata("42", None))),
        (6, json_response(&directory(false, None))),
    ] {
        let mut bodies = flow("42", false, &r);
        bodies[at] = bad;
        bodies.truncate(at + 1);
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_update_playlist(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        fixture::requests(&mut f, at + 1).await;
    }
    for (field, value) in [
        ("desc", json!("changed")),
        ("tag", json!("其他,安静")),
        ("tagid", json!("900,400")),
        ("pic", json!(BIG)),
        ("total", json!(3)),
        ("uid", json!(7)),
        ("igsl", json!("1")),
    ] {
        let mut m = metadata("42", Some(&r));
        m["sl_data"][field] = value;
        let mut bodies = flow("42", false, &r);
        bodies[5] = json_response(&m);
        bodies.truncate(6);
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_update_playlist(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        fixture::requests(&mut f, 6).await;
    }
    for (field, value) in [
        ("ispub", json!(true)),
        ("pic", json!(BIG)),
        ("musicnum", json!(3)),
        ("type", json!("RADIO")),
    ] {
        let mut d = directory(false, Some(&r));
        d["plist"][0][field] = value;
        let mut bodies = flow("42", false, &r);
        bodies[6] = json_response(&d);
        let mut f = fixture::setup(bodies).await;
        assert_eq!(
            f.client
                .native_update_playlist(&credential(), "101", &r)
                .await
                .unwrap_err()
                .details["write_outcome"],
            "unconfirmed"
        );
        fixture::requests(&mut f, 7).await;
    }
}

#[test]
fn native_edit_typed_metadata_rejects_duplicate_fields_errors_and_reflections() {
    let input = credential::NativeCredential::parse(&credential())
        .unwrap()
        .input()
        .unwrap();
    let good = metadata("42", None).to_string();
    for bad in [
        good.replace("\"title\":", "\"title\":\"other\",\"title\":"),
        good.replace("\"sl_data\":", "\"sl_data\":{},\"sl_data\":"),
        good.replace("{\"sl_data\":", "{\"errcode\":603,\"sl_data\":"),
        good.replace("流行", "selected-session"),
    ] {
        assert!(parse_metadata(bad.as_bytes(), &input, "101").is_err());
    }
}

#[tokio::test]
async fn native_submission_repro_metadata_edit_requires_explicit_review_management() {
    let r = request(None);
    let mut bodies = flow("42", true, &r);
    let mut before = metadata("42", None);
    before["sl_data"]["igsl"] = json!("1");
    let mut after = metadata("42", Some(&r));
    after["sl_data"]["igsl"] = json!("1");
    bodies[2] = json_response(&before);
    bodies[5] = json_response(&after);
    bodies.truncate(3);
    let mut f = fixture::setup(bodies).await;
    let result = f
        .client
        .native_update_playlist(&credential(), "101", &r)
        .await;
    assert_eq!(
        result.err().map(|e| e.code),
        Some(ErrorCode::CapabilityNotSupported)
    );
    let seen = fixture::requests(&mut f, 3).await;
    assert!(
        seen.iter()
            .all(|request| request.starts_with("GET ") && !request.contains("user_songlist_up"))
    );
}
