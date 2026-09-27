use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use tokio::sync::Notify;

const SID: &str = "private-profile-session";
pub(crate) fn body(uid: &str) -> serde_json::Value {
    json!({"status":200,"info":[{
        "UID":uid,"NICK_NAME":"听众 + %20","PIC":"https://img1.kuwo.cn/star/userhead/synthetic.jpg",
        "SIGNATURE":"Music\n第二行","LEVEL":"3","BIRTHDAY":"2000-02-29","REGTM":"2020-01-02 03:04:05",
        "FIELD6":"http://img2.kuwo.cn/star/userhead/background.jpg",
        "PASSWORD_ANSWER":"private-answer","PASSWORD_QUESTION":"private-question",
        "PWD_EMAIL":"private-mail@example.test","PWD_PHONE":"13800000000","QQ":"private-qq",
        "NAME":"private-login-name"
    }],"follow_relation":false,"follow_cnt":999,"fans_cnt":999})
}
pub(crate) fn reply(uid: &str) -> Vec<u8> {
    fixture::encrypted(&body(uid))
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", SID).caller().unwrap()
}
fn input() -> KuwoNativeSessionInput {
    credential::NativeCredential::parse(&credential())
        .unwrap()
        .input()
        .unwrap()
}
fn parsed(value: &serde_json::Value) -> Result<UserProfile> {
    parse(&serde_json::to_vec(value).unwrap(), &input())
}

#[tokio::test]
async fn self_profile_validates_identity_then_sends_exact_native_query_and_metadata() {
    let encrypted = codec::fixture_response(&serde_json::to_vec(&body("42")).unwrap(), b"17894932");
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        response(
            200,
            "text/html; charset=utf-8",
            "Set-Cookie: alien=ignored\r\n",
            &encrypted,
        ),
    ])
    .await;
    let profile = f.client.native_self_profile(&credential()).await.unwrap();
    assert_eq!(profile.user.name, "听众 + %20");
    assert_eq!(profile.user.signature.as_deref(), Some("Music\n第二行"));
    assert_eq!(profile.level, Some(3));
    assert_eq!(profile.birthday.as_deref(), Some("2000-02-29"));
    assert_eq!(profile.created_at.as_deref(), Some("2020-01-02 03:04:05"));
    assert!(
        profile
            .background_url
            .as_deref()
            .unwrap()
            .starts_with("http://")
    );
    assert!(profile.follower_count.is_none());
    assert!(profile.user.followed.is_none());
    let serialized = serde_json::to_string(&profile).unwrap();
    for forbidden in [
        SID,
        "private-answer",
        "private-question",
        "private-mail",
        "13800000000",
        "private-qq",
        "private-login-name",
        "ignored",
        "follow_cnt",
    ] {
        assert!(!serialized.contains(forbidden), "{forbidden}");
    }
    let requests = fixture::requests(&mut f, 2).await;
    assert!(requests[0].starts_with("GET /u.s?"));
    assert!(requests[0].contains("uid=42&sid=private-profile-session"));
    let vector: serde_json::Value = serde_json::from_str(include_str!("test_vector.json")).unwrap();
    assert_eq!(
        query(&input(), b"17894932"),
        vector["plain"].as_str().unwrap()
    );
    assert!(requests[1].starts_with(&format!(
        "GET {PATH}?f=ar&q={} HTTP/1.1\r\n",
        vector["query"].as_str().unwrap()
    )));
    let header = requests[1]
        .lines()
        .find(|line| line.starts_with("cookies:"))
        .unwrap();
    assert!(header.contains("loginUid=42,loginSid=private-profile-session,appUid=1234567890,"));
    assert!(!requests[1].to_lowercase().contains("\r\ncookie:"));
    assert!(!requests[1].contains("alien"));
}

#[test]
fn sparse_profiles_keep_unknown_fields_unknown_and_never_use_login_name_as_nickname() {
    let value =
        parsed(&json!({"status":"200","info":[{"UID":42,"LEVEL":0,"NAME":"private-login"}]}))
            .unwrap();
    assert!(value.user.name.is_empty());
    assert_eq!(value.level, Some(0));
    assert!(value.user.avatar_url.is_none());
    assert!(value.birthday.is_none());
    let mut v = body("42");
    v["info"][0]["FIELD7"] = json!("123");
    v["info"][0]["FIELD6"] = json!("not-a-preset-url");
    assert!(parsed(&v).unwrap().background_url.is_none());
}

#[test]
fn profile_accepts_the_official_star_host_for_avatar_metadata() {
    let mut value = body("42");
    value["info"][0]["PIC"] = json!("https://star.kuwo.cn/star/userhead/synthetic.jpg");
    let profile = parsed(&value).unwrap();
    assert_eq!(
        profile.user.avatar_url.as_deref(),
        Some("https://star.kuwo.cn/star/userhead/synthetic.jpg")
    );
}

#[test]
fn profile_rejects_malformed_or_ambiguous_identity_fields_and_secret_reflections() {
    let mut bad = vec![
        json!({}),
        json!({"status":200}),
        json!({"status":200,"info":[]}),
        json!({"status":500,"info":[{"UID":42}]}),
        json!({"status":200,"info":[{"UID":42},{"UID":42}]}),
    ];
    for (field, value) in [
        ("UID", json!("43")),
        ("UID", json!("042")),
        ("UID", json!(null)),
        ("NICK_NAME", json!(3)),
        ("NICK_NAME", json!("bad\u{0}name")),
        ("NICK_NAME", json!("x".repeat(1025))),
        ("NICK_NAME", json!(SID)),
        ("SIGNATURE", json!("x=private%2Dprofile%2Dsession")),
        ("SIGNATURE", json!("x".repeat(8193))),
        ("LEVEL", json!(-1)),
        ("LEVEL", json!("01")),
        ("LEVEL", json!(4294967296_u64)),
        ("BIRTHDAY", json!(true)),
        ("REGTM", json!("private-profile-session")),
        ("PIC", json!("https://example.test/avatar.jpg")),
        ("PIC", json!("https://img1.kuwo.cn@127.0.0.1/a")),
        (
            "PIC",
            json!("https://img1.kuwo.cn/a?sid=private-profile-session"),
        ),
        ("PIC", json!("file:///tmp/avatar")),
        ("FIELD6", json!("https://img1.kuwo.cn/a#fragment")),
    ] {
        let mut v = body("42");
        v["info"][0][field] = value;
        bad.push(v);
    }
    for v in bad {
        let error = parsed(&v).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains(SID));
    }
    for raw in [
        r#"{"status":200,"status":200,"info":[{"UID":42}]}"#,
        r#"{"status":200,"info":[{"UID":42,"UID":43}]}"#,
        r#"{"status":200,"info":[{"UID":42,"NICK_NAME":"a","NICK_NAME":"b"}]}"#,
    ] {
        assert!(parse(raw.as_bytes(), &input()).is_err());
    }
}

#[tokio::test]
async fn invalid_validation_and_profile_errors_never_fall_back_retry_or_expose_partial_data() {
    let mut f = fixture::setup(vec![json_response(
        &json!({"result":"fail","reason":"error_user_invalid"}),
    )])
    .await;
    assert_eq!(
        f.client
            .native_self_profile(&credential())
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    fixture::requests(&mut f, 1).await;
    for reply in [
        fixture::encrypted(&json!({"status":500,"msg":SID})),
        reply("43"),
        response(
            200,
            "text/html",
            "",
            b"<html>private-profile-session</html>",
        ),
        response(
            302,
            "application/json",
            "Location: https://example.test/\r\n",
            b"",
        ),
        response(
            429,
            "application/json",
            "Retry-After: 900\r\n",
            b"private-profile-session",
        ),
    ] {
        let mut f = fixture::setup(vec![json_response(&json!({"result":"ok"})), reply]).await;
        let e = f
            .client
            .native_self_profile(&credential())
            .await
            .unwrap_err();
        assert!(!format!("{e:?}").contains(SID));
        fixture::requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn profile_sdk_rejects_metadata_injection_before_network() {
    let f = fixture::setup(vec![]).await;
    let credential = fixture::credential_fixture("42", "session,loginUid=43")
        .caller()
        .unwrap();
    assert_eq!(
        f.client
            .native_self_profile(&credential)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn profile_sdk_cancel_and_timeout_at_either_boundary_return_no_profile() {
    for boundary in 0..2 {
        for cancel in [false, true] {
            let gate = Arc::new(Notify::new());
            let bodies = [json_response(&json!({"result":"ok"})), reply("42")];
            let mut f = fixture::setup_gated(
                bodies
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect(),
            )
            .await;
            let client = f.client.clone();
            let task = tokio::spawn(async move { client.native_self_profile(&credential()).await });
            for _ in 0..=boundary {
                tokio::time::timeout(Duration::from_secs(3), f.seen.recv())
                    .await
                    .unwrap()
                    .unwrap();
            }
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(
                    task.await.unwrap().unwrap_err().code,
                    ErrorCode::UpstreamTimeout
                );
            }
            assert!(f.seen.try_recv().is_err());
        }
    }
}
