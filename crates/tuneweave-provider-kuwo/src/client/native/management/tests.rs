use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

pub(crate) const NAME: &str = "新歌单 + %20 & \"保留\"";
pub(crate) fn directory(ids: &[u64]) -> Value {
    let mut items=ids.iter().map(|id|json!({"id":id,"type":"GENERAL","title":if *id==201 {NAME}else{"现有歌单"},"musicnum":0,"ispub":true})).collect::<Vec<_>>();
    items.extend([
        json!({"id":901,"type":"MYFAVORITE","musicnum":0,"ispub":false}),
        json!({"id":700,"type":"RADIO"}),
    ]);
    json!({"errcode":0,"plist":items})
}
pub(crate) fn flow(create: bool) -> Vec<Vec<u8>> {
    vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(&[101, 102])),
        json_response(&if create {
            json!({"errcode":0,"pid":201})
        } else {
            json!({"errcode":0})
        }),
        json_response(&directory(if create { &[101, 102, 201] } else { &[] })),
    ]
}
pub(crate) fn create(account: Option<&str>) -> PlaylistCreateRequest {
    PlaylistCreateRequest {
        account: account.map(str::to_owned),
        ..PlaylistCreateRequest::new(NAME)
    }
}
pub(crate) fn delete(ids: &[&str], account: Option<&str>) -> PlaylistDeleteRequest {
    PlaylistDeleteRequest {
        playlist_refs: ids
            .iter()
            .map(|id| ResourceRef::new(Platform::Kuwo, *id).unwrap())
            .collect(),
        account: account.map(str::to_owned),
    }
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[test]
fn native_management_creation_matches_official_utf16_name_width_limit() {
    for accepted in [
        "中".repeat(20),
        "a".repeat(40),
        "中".repeat(19) + "ab",
        "😀".repeat(20),
    ] {
        let request = PlaylistCreateRequest::new(accepted);
        assert!(Mutation::Create(&request).validate().is_ok());
    }

    for rejected in [
        "中".repeat(21),
        "a".repeat(41),
        "中".repeat(19) + "abc",
        "😀".repeat(21),
    ] {
        let request = PlaylistCreateRequest::new(rejected);
        assert_eq!(
            Mutation::Create(&request).validate().unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
}

async fn run(client: &KuwoClient, is_create: bool) -> Result<Value> {
    if is_create {
        client
            .native_create_playlist(&credential(), &create(None))
            .await
            .map(|v| serde_json::to_value(v).unwrap())
    } else {
        client
            .native_delete_playlists(&credential(), &delete(&["101", "102"], None))
            .await
            .map(|v| serde_json::to_value(v).unwrap())
    }
}

#[tokio::test]
async fn native_management_sdk_uses_exact_single_post_and_confirms_selected_account_readback() {
    for is_create in [true, false] {
        let mut replies = flow(is_create);
        replies[2] = response(
            200,
            "application/json",
            "Set-Cookie: alien=never-send; Path=/\r\n",
            &serde_json::to_vec(&if is_create {
                json!({"errcode":0,"pid":"201","token":"never-export"})
            } else {
                json!({"errcode":0})
            })
            .unwrap(),
        );
        let mut f = fixture::setup(replies).await;
        let result = run(&f.client, is_create).await.unwrap();
        assert_eq!(result["extensions"]["confirmed"], true);
        assert_eq!(result["extensions"]["atomic"], false);
        assert_eq!(result["extensions"]["library_owner_id"], "42");
        if is_create {
            assert_eq!(result["playlist_ref"], "kuwo:201");
            assert_eq!(result["playlist"]["name"], NAME);
        } else {
            assert_eq!(result["playlist_refs"], json!(["kuwo:101", "kuwo:102"]));
        }
        let encoded = result.to_string();
        for secret in ["selected-session", "never-export", "Cookies", "loginSid"] {
            assert!(!encoded.contains(secret));
        }
        let seen = fixture::requests(&mut f, 4).await;
        assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 1);
        let (head, body) = seen[2].split_once("\r\n\r\n").unwrap();
        assert!(
            head.to_lowercase()
                .contains("\r\ncontent-type: application/x-www-form-urlencoded\r\n")
        );
        assert!(!head.to_lowercase().contains("\r\ncookie:"));
        assert!(head.contains("loginUid=42,loginSid=selected-session,"));
        let url = Url::parse(&format!(
            "https://fixture.test{}",
            head.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        assert_eq!(url.path(), "/pl.svc");
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let map = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(map.len(), pairs.len());
        assert_eq!(
            map["op"],
            if is_create {
                "pl3_addlist"
            } else {
                "pl3_deletelist"
            }
        );
        for (key, value) in [
            ("uid", "42"),
            ("sid", "selected-session"),
            ("recommend", "1"),
            ("encode", "utf-8"),
            ("plat", "ar"),
            ("prod", "kwplayer_ar_12.2.2.0"),
            ("source", CLIENT_SOURCE),
        ] {
            assert_eq!(map[key], value);
        }
        let actual: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            actual,
            if is_create {
                json!({"title":NAME,"tag":"","pic":"","intro":"","data":[],"ispub":true,"turn":0})
            } else {
                json!({"plist":[101,102]})
            }
        );
        for r in &seen {
            assert!(!r.contains("alien"));
            assert!(!r.contains("ucheck"));
        }
    }
}

#[tokio::test]
async fn native_management_validates_all_input_before_network_and_all_targets_before_write() {
    let mut f = fixture::setup(vec![]).await;
    for name in ["", "  ", "bad\nname", &"x".repeat(1025)] {
        let r = PlaylistCreateRequest::new(name);
        assert_eq!(
            f.client
                .native_create_playlist(&credential(), &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (visibility, kind) in [
        (PlaylistVisibility::Private, PlaylistKind::Normal),
        (PlaylistVisibility::PlatformDefault, PlaylistKind::Normal),
        (PlaylistVisibility::Public, PlaylistKind::Video),
        (PlaylistVisibility::Public, PlaylistKind::Shared),
    ] {
        let mut r = create(None);
        r.visibility = visibility;
        r.kind = kind;
        assert_eq!(
            f.client
                .native_create_playlist(&credential(), &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }
    assert!(
        f.client
            .native_create_playlist(&credential(), &create(Some("personal")))
            .await
            .is_err()
    );
    for ids in [
        vec![],
        vec!["101", "101"],
        vec!["0"],
        vec!["0101"],
        vec!["9223372036854775808"],
        vec!["1"; 101],
    ] {
        assert_eq!(
            f.client
                .native_delete_playlists(&credential(), &delete(&ids, None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut other = delete(&["101"], None);
    other.playlist_refs[0] = ResourceRef::new(Platform::Kugou, "101").unwrap();
    assert!(
        f.client
            .native_delete_playlists(&credential(), &other)
            .await
            .is_err()
    );
    fixture::requests(&mut f, 0).await;
    for ids in [["101", "901"], ["101", "700"], ["101", "999"]] {
        let mut f = fixture::setup(flow(false)[..2].to_vec()).await;
        let error = f
            .client
            .native_delete_playlists(&credential(), &delete(&ids, None))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert!(error.details.get("write_outcome").is_none());
        assert!(
            fixture::requests(&mut f, 2)
                .await
                .iter()
                .all(|r| r.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn native_management_ack_never_accepts_queue_retirement_codes_or_invented_ids() {
    let mut invalids = vec![
        json!({}),
        json!({"errcode":0,"result":"fail","pid":201}),
        json!({"errcode":0,"uid":7,"pid":201}),
        json!({"errcode":false,"pid":201}),
    ];
    invalids.extend(
        [603, 604, 605, 606, 607, 614]
            .map(|code| json!({"errcode":code,"pid":201,"message":"selected-session"})),
    );
    for is_create in [true, false] {
        let mut bodies = invalids.iter().map(json_response).collect::<Vec<_>>();
        bodies.push(response(
            200,
            "application/json",
            "",
            br#"{"errcode":0,"errcode":603,"pid":201}"#,
        ));
        if is_create {
            bodies.extend(
                [
                    json!({"errcode":0}),
                    json!({"errcode":0,"pid":0}),
                    json!({"errcode":0,"pid":"0201"}),
                    json!({"errcode":0,"pid":-1}),
                ]
                .iter()
                .map(json_response),
            );
        }
        for body in bodies {
            let mut replies = flow(is_create)[..3].to_vec();
            replies[2] = body;
            let mut f = fixture::setup(replies).await;
            let error = run(&f.client, is_create).await.unwrap_err();
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            assert!(!error.retryable);
            assert!(!format!("{error:?}").contains("selected-session"));
            fixture::requests(&mut f, 3).await;
        }
    }
}

#[tokio::test]
async fn native_management_http_failure_and_invalid_response_do_not_retry_or_readback() {
    let bodies = vec![
        response(
            302,
            "application/json",
            "Location: https://evil.test/\r\n",
            b"{}",
        ),
        response(401, "application/json", "", b"{}"),
        response(429, "application/json", "Retry-After: 5\r\n", b"{}"),
        response(200, "text/html", "", b"{}"),
        response(200, "application/json", "", &vec![b' '; ACK_LIMIT + 1]),
    ];
    for body in bodies {
        let mut replies = flow(true)[..3].to_vec();
        replies[2] = body;
        let mut f = fixture::setup(replies).await;
        let error = run(&f.client, true).await.unwrap_err();
        assert_eq!(error.details["write_requests_dispatched"], 1);
        assert!(!error.retryable);
        fixture::requests(&mut f, 3).await;
    }
}

#[tokio::test]
async fn native_management_creation_requires_new_ordinary_public_empty_named_playlist() {
    for id in [101, 700, 901] {
        let mut replies = flow(true)[..3].to_vec();
        replies[2] = json_response(&json!({"errcode":0,"pid":id}));
        let mut f = fixture::setup(replies).await;
        assert_eq!(
            run(&f.client, true).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        fixture::requests(&mut f, 3).await;
    }
    for (key, value) in [
        ("id", json!(202)),
        ("title", json!("different")),
        ("musicnum", json!(1)),
        ("musicnum", Value::Null),
        ("ispub", json!(false)),
        ("ispub", Value::Null),
        ("type", json!("RADIO")),
    ] {
        let mut after = directory(&[101, 102, 201]);
        after["plist"][2][key] = value;
        let mut replies = flow(true);
        replies[3] = json_response(&after);
        let mut f = fixture::setup(replies).await;
        let e = run(&f.client, true).await.unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        fixture::requests(&mut f, 4).await;
    }
}

#[tokio::test]
async fn native_management_deletion_must_remove_every_id_even_if_kind_changes() {
    for kind in ["GENERAL", "RADIO", "ORDER", "PC_DEFAULT"] {
        let mut after = directory(&[101]);
        after["plist"][0]["type"] = json!(kind);
        let mut replies = flow(false);
        replies[3] = json_response(&after);
        let mut f = fixture::setup(replies).await;
        let e = run(&f.client, false).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        fixture::requests(&mut f, 4).await;
    }
}
