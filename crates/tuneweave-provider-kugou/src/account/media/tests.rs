use super::*;
use crate::account::tests::{frame, ok, server, session};

const HASH: &str = "ABCDEF0123456789ABCDEF0123456789";
pub(crate) fn tracker_response(value: Value) -> TrackerResponse {
    TrackerResponse {
        bytes: value.to_string().into_bytes(),
        secrets: vec![
            "synthetic-login-secret".into(),
            "synthetic-user-auth".into(),
            "synthetic-song-auth".into(),
        ],
    }
}
fn user(session: &NativeSession) -> UserAuthorization {
    UserAuthorization {
        session: session.clone(),
        auth: "synthetic-user-auth".into(),
    }
}
fn song(session: &NativeSession) -> SongAuthorization {
    SongAuthorization {
        user: user(session),
        id: 901,
        hash: HASH.to_ascii_lowercase(),
        auth: "synthetic-song-auth".into(),
        open_time: "1700000000".into(),
    }
}
fn query(raw: &str, path: &str, kind: KugouLoginClient) -> BTreeMap<String, String> {
    assert!(raw.starts_with("GET "));
    let target = raw
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    assert_eq!(url.path(), path);
    let mut query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    let signature = query.remove("signature").unwrap();
    let signed = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(
        signature,
        match kind {
            KugouLoginClient::Standard => android_signature(&signed, &[]),
            KugouLoginClient::Concept => concept_signature(&signed, &[]),
            _ => panic!(),
        }
    );
    assert_eq!(query["appid"], kind.appid().to_string());
    assert_eq!(
        query["clientver"],
        if path == "/v1/user_verify" {
            kind.clientver().to_string()
        } else {
            "11561".into()
        }
    );
    assert_eq!(query["module_id"], "51");
    assert_eq!(query["userid"], "123456789");
    assert_eq!(query["token"], "synthetic-original-token");
    assert_eq!(query["uuid"], "-");
    assert_eq!(raw.split_once("\r\n\r\n").unwrap().1, "");
    let head = raw.split_once("\r\n").unwrap().1.to_ascii_lowercase();
    for forbidden in [
        "cookie:",
        "authorization:",
        "x-router:",
        "x-forwarded-for:",
        "x-real-ip:",
    ] {
        assert!(!head.contains(forbidden));
    }
    query
}

#[tokio::test]
async fn native_media_auth_chain_uses_exact_clients_session_device_and_separate_download_behavior()
{
    for kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for behavior in [Behavior::Play, Behavior::Download] {
            let session = session(kind);
            let (client, requests) = server(vec![ok(json!({"auth":"synthetic-user-auth","userid":123456789,"module_id":51})),
                ok(json!({"auth":"synthetic-song-auth","open_time":"1700000000","album_audio_id":901,"hash":HASH})),
                frame(200,"Content-Type: application/json\r\n",br#"{"status":1}"#.to_vec())]).await;
            let user = client.native_user_authorization(&session).await.unwrap();
            let song = client
                .native_song_authorization(&session, user, 901, HASH)
                .await
                .unwrap();
            let response = client
                .native_audio_tracker(&session, song, 70, "320", behavior)
                .await
                .unwrap();
            for secret in [
                "synthetic-original-token",
                "synthetic-user-auth",
                "synthetic-song-auth",
                "1700000000",
            ] {
                assert!(
                    response
                        .check_url(&format!("https://fs.kugou.com/a?grant={secret}"))
                        .is_err()
                );
            }
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 3);
            for (index, path) in ["/v1/user_verify", "/v1/authorization", "/tracker/v5/url"]
                .into_iter()
                .enumerate()
            {
                let p = query(&requests[index], path, kind);
                assert_eq!(p["dfid"], session.device.dfid());
                assert_eq!(p["mid"], session.device.mid);
                assert!(!p.contains_key("IsFreePart"));
                if index == 0 {
                    assert_eq!(p.len(), 9);
                }
                if index == 1 {
                    assert_eq!(p["authorization"], "synthetic-user-auth");
                    assert_eq!(p["hash"], HASH.to_ascii_lowercase());
                    assert_eq!(p["album_audio_id"], "901");
                    assert!(!p.contains_key("auth"));
                }
                if index == 2 {
                    assert_eq!(p["behavior"], behavior.name());
                    assert_eq!(p["auth"], "synthetic-song-auth");
                    assert_eq!(p["open_time"], "1700000000");
                    assert_eq!(p["quality"], "320");
                    assert_eq!(p["album_id"], "70");
                    assert_eq!(p["album_audio_id"], "901");
                    assert_eq!(
                        p["pid"],
                        if kind == KugouLoginClient::Concept {
                            "411"
                        } else {
                            "2"
                        }
                    );
                    assert_eq!(
                        p["page_id"],
                        if kind == KugouLoginClient::Concept {
                            "967177915"
                        } else {
                            "151369488"
                        }
                    );
                    assert_eq!(p["version"], "11430");
                    assert!(!p.contains_key("authorization"));
                    let salt = if kind == KugouLoginClient::Concept {
                        "185672dd44712f60bb1736df5a377e82"
                    } else {
                        "57ae12eb6890223e355ccfcb74edf70d"
                    };
                    assert_eq!(
                        p["key"],
                        format!(
                            "{:x}",
                            Md5::digest(format!(
                                "{}{salt}{}{}123456789",
                                HASH.to_ascii_lowercase(),
                                kind.appid(),
                                session.device.mid
                            ))
                        )
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn single_song_grants_require_both_fields_and_matching_echoed_identity() {
    let session = session(KugouLoginClient::Standard);
    for (key, value, code) in [
        ("auth", json!(""), ErrorCode::UpstreamError),
        ("auth", json!(null), ErrorCode::UpstreamError),
        ("open_time", json!(null), ErrorCode::UpstreamError),
        ("open_time", json!(0), ErrorCode::UpstreamError),
        ("open_time", json!(true), ErrorCode::UpstreamError),
        ("userid", json!(999), ErrorCode::Conflict),
        ("module_id", json!(52), ErrorCode::Conflict),
        ("album_audio_id", json!(902), ErrorCode::Conflict),
        ("hash", json!("other"), ErrorCode::Conflict),
    ] {
        let mut value0 = json!({"auth":"synthetic-song-auth","open_time":1700000000});
        value0[key] = value;
        let (client, requests) = server(vec![ok(value0)]).await;
        let error = client
            .native_song_authorization(&session, user(&session), 901, HASH)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, code, "{key}");
        assert!(!format!("{error:?}").contains("synthetic-song-auth"));
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn authorization_is_not_reusable_across_session_rotations_devices_or_accounts() {
    let original = session(KugouLoginClient::Standard);
    for change in ["token", "uid", "device", "client"] {
        let mut next = original.clone();
        match change {
            "token" => next.token = "new-session-token".into(),
            "uid" => next.user_id = "222".into(),
            "device" => next.device = crate::device::KugouDevice::default().identity(),
            _ => next.client = KugouLoginClient::Concept,
        }
        let (client, requests) = server(vec![]).await;
        assert_eq!(
            client
                .native_song_authorization(&next, user(&original), 901, HASH)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            client
                .native_audio_tracker(&next, song(&original), 70, "128", Behavior::Play)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        assert!(requests.await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn account_auth_transport_never_follows_redirects_and_bounds_response_types_and_size() {
    for (status, headers, body, code) in [
        (
            302,
            "Location: http://127.0.0.1:1/leak\r\n",
            b"".to_vec(),
            ErrorCode::UpstreamError,
        ),
        (
            401,
            "Content-Type: application/json\r\n",
            b"{}".to_vec(),
            ErrorCode::AuthenticationRequired,
        ),
        (
            429,
            "Retry-After: 10\r\n",
            b"".to_vec(),
            ErrorCode::RateLimited,
        ),
        (
            200,
            "Content-Type: text/html\r\n",
            b"<html>".to_vec(),
            ErrorCode::UpstreamError,
        ),
        (
            200,
            "Content-Type: application/json\r\n",
            vec![b' '; RESPONSE_LIMIT + 1],
            ErrorCode::UpstreamError,
        ),
        (
            200,
            "Content-Type: application/json\r\nssa-code: 400\r\n",
            b"{}".to_vec(),
            ErrorCode::PermissionDenied,
        ),
    ] {
        let (client, requests) = server(vec![frame(status, headers, body)]).await;
        assert_eq!(
            client
                .native_user_authorization(&session(KugouLoginClient::Standard))
                .await
                .err()
                .unwrap()
                .code,
            code
        );
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[test]
fn auth_status_rejections_do_not_parse_or_export_untrusted_success_payloads() {
    let session = session(KugouLoginClient::Standard);
    for (body, code) in [
        (
            json!({"status":0,"error_code":20017,"data":"synthetic-user-auth"}),
            ErrorCode::AuthenticationRequired,
        ),
        (
            json!({"status":0,"error_code":35002,"data":null}),
            ErrorCode::PermissionDenied,
        ),
        (
            json!({"status":1,"error_code":0,"errcode":20017,"data":{}}),
            ErrorCode::UpstreamError,
        ),
        (
            json!({"status":1,"data":{"auth":"secret\nunsafe"}}),
            ErrorCode::UpstreamError,
        ),
        (
            json!({"status":1,"data":{"auth":"secret","userid":"0111"}}),
            ErrorCode::UpstreamError,
        ),
    ] {
        let error = auth_data(body.to_string().as_bytes(), &session)
            .err()
            .unwrap();
        assert_eq!(error.code, code);
        assert!(!format!("{error:?}").contains("synthetic-user-auth"));
    }
    assert!(
        auth_data(
            br#"{"status":1,"data":{"auth":"first","auth":"second"}}"#,
            &session
        )
        .is_err()
    );
}
