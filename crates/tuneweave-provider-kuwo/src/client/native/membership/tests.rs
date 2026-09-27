use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use num_bigint::BigUint;
use tokio::sync::Notify;

const NOW: u64 = 1_700_000_000_000;
const SID: &str = "private-membership-session";
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", SID).caller().unwrap()
}
fn input() -> KuwoNativeSessionInput {
    credential::NativeCredential::parse(&credential())
        .unwrap()
        .input()
        .unwrap()
}
fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("test_vectors.json")).unwrap()
}
pub(crate) fn body() -> serde_json::Value {
    serde_json::from_str(vectors()["plain"].as_str().unwrap()).unwrap()
}
pub(crate) fn reply() -> Vec<u8> {
    encrypted(&body())
}
pub(crate) fn encrypted(value: &serde_json::Value) -> Vec<u8> {
    response(
        200,
        "text/html;charset=UTF-8",
        "Set-Cookie: alien=ignored\r\n",
        &crypto::encrypt(&serde_json::to_vec(value).unwrap()),
    )
}
fn parsed(value: &serde_json::Value) -> Result<MembershipSummary> {
    parse(&serde_json::to_vec(value).unwrap(), &input())
}

#[test]
fn independent_rsa_vectors_match_pkcs1_padding_framing_and_anonymous_empty_data_is_not_membership()
{
    let v = vectors();
    let cipher = v["ciphertext"].as_str().unwrap();
    let plain = crypto::decode(cipher.as_bytes()).unwrap();
    assert_eq!(plain, v["plain"].as_str().unwrap().as_bytes());
    let wrapped = format!(" \r\n{}\r\n{}\t", &cipher[..80], &cipher[80..]);
    assert_eq!(crypto::decode(wrapped.as_bytes()).unwrap(), plain);
    let anon =
        crypto::decode(v["anonymous_empty_ciphertext"].as_str().unwrap().as_bytes()).unwrap();
    assert!(parse(&anon, &input()).is_err());
    assert!(parsed(&json!({"meta":{"code":200},"ctime":NOW,"data":{}})).is_err());
}

#[test]
fn rsa_rejects_bad_base64_frames_limits_and_invalid_padding() {
    let v = vectors();
    let cipher = BASE64_STANDARD
        .decode(v["ciphertext"].as_str().unwrap())
        .unwrap();
    let mut bad = vec![
        vec![],
        b"<html>bad</html>".to_vec(),
        vec![b'A'; 65537],
        BASE64_STANDARD.encode([0_u8; 127]).into_bytes(),
        BASE64_STANDARD.encode([0_u8; 129]).into_bytes(),
    ];
    let mut malformed = cipher.clone();
    malformed[128] = b'!';
    bad.push(BASE64_STANDARD.encode(malformed).into_bytes());
    let mut trailing = cipher;
    trailing.extend_from_slice(b"#PART#");
    bad.push(BASE64_STANDARD.encode(trailing).into_bytes());
    let mut oversized = Vec::new();
    for i in 0..257 {
        if i > 0 {
            oversized.extend_from_slice(b"#PART#");
        }
        oversized.extend_from_slice(&[0; 128]);
    }
    bad.push(BASE64_STANDARD.encode(oversized).into_bytes());
    let n = BigUint::parse_bytes(key::MODULUS.as_bytes(), 16).unwrap();
    bad.push(BASE64_STANDARD.encode(n.to_bytes_be()).into_bytes());
    for kind in 0..5 {
        let mut block = [0x35_u8; 128];
        block[0] = 0;
        block[1] = 2;
        block[12] = 0;
        match kind {
            0 => block[1] = 1,
            1 => block[2] = 0,
            2 => block[9] = 0,
            3 => block[12] = 0x35,
            _ => {
                block[12] = 0x35;
                block[127] = 0;
            }
        }
        let bytes = BigUint::from_bytes_be(&block)
            .modpow(&BigUint::from(65537_u32), &n)
            .to_bytes_be();
        let mut encrypted = vec![0; 128 - bytes.len()];
        encrypted.extend(bytes);
        bad.push(BASE64_STANDARD.encode(encrypted).into_bytes());
    }
    for wire in bad {
        assert!(crypto::decode(&wire).is_err());
    }
}

#[test]
fn membership_maps_separate_products_server_time_expiry_and_literal_classification_without_media_rights()
 {
    let summary = parsed(&body()).unwrap();
    assert_eq!(summary.user_ref.as_ref().unwrap().id(), "42");
    assert_eq!(summary.active, Some(true));
    assert!(summary.level.is_none());
    assert!(summary.annual_count.is_none());
    assert!(summary.expires_at.is_none());
    let entries = summary.extensions["memberships"].as_array().unwrap();
    assert_eq!(entries.len(), 8);
    assert_eq!(entries[1]["kind"], "music");
    assert_eq!(entries[1]["auto_renew"], true);
    assert_eq!(entries[1]["expires_at"], "2023-11-15T22:13:20.000Z");
    assert_eq!(entries[4]["state"], "expired");
    assert_eq!(entries[0]["state"], "none");
    assert_eq!(summary.extensions["server_time_ms"], NOW);
    assert_eq!(summary.extensions["annual_user_code"], 1);
    let output = serde_json::to_string(&summary).unwrap();
    for forbidden in [
        SID,
        "never-export-phone",
        "never-export-token",
        "ignored",
        "playable",
        "downloadable",
    ] {
        assert!(!output.contains(forbidden));
    }
    let mut one = body();
    one["data"]["vipLuxuryExpire"] = json!(0);
    assert_eq!(
        parsed(&one).unwrap().expires_at.as_deref(),
        Some("2023-11-15T22:13:20.000Z")
    );
}

#[test]
fn unknown_and_expired_memberships_do_not_become_false_or_active_from_unrelated_products() {
    let keys = [
        "vipExpire",
        "vipmExpire",
        "vipLuxuryExpire",
        "svipExpire",
        "chezaiExpire",
        "experienceExpire",
        "vipAdExpire",
        "vip3Expire",
    ];
    let mut value = body();
    for k in keys {
        value["data"][k] = json!(0);
    }
    value["data"]["vipAdExpire"] = json!(NOW + 1000);
    assert_eq!(parsed(&value).unwrap().active, Some(false));
    value["data"].as_object_mut().unwrap().remove("svipExpire");
    assert_eq!(parsed(&value).unwrap().active, None);
    value["data"]["vipmExpire"] = json!(NOW + 1000);
    let partial = parsed(&value).unwrap();
    assert_eq!(partial.active, Some(true));
    assert!(partial.expires_at.is_none());
    value["data"]["vipmExpire"] = json!(NOW);
    value["data"]["svipExpire"] = json!(0);
    let expired = parsed(&value).unwrap();
    assert_eq!(expired.active, Some(false));
    assert_eq!(expired.extensions["memberships"][1]["state"], "expired");
    let minimal =
        parsed(&json!({"meta":{"code":"200"},"ctime":NOW.to_string(),"data":{"vipmExpire":0}}))
            .unwrap();
    assert_eq!(minimal.active, None);
    assert!(minimal.extensions["memberships"][1]["auto_renew"].is_null());

    let mut blank = body();
    for field in [
        "vipExpire",
        "vipmExpire",
        "vipLuxuryExpire",
        "svipExpire",
        "chezaiExpire",
        "experienceExpire",
        "vipAdExpire",
        "vip3Expire",
        "vipmAutoPayUser",
        "luxAutoPayUser",
        "svipAutoPayUser",
        "cheZaiAutoPayUser",
    ] {
        blank["data"][field] = json!("");
    }
    let unknown = parsed(&blank).unwrap();
    assert_eq!(unknown.active, None);
    assert!(
        unknown.extensions["memberships"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["state"] == "unknown")
    );
    assert!(parsed(&json!({"meta":{"code":200},"ctime":NOW,"data":{}})).is_err());
}

#[test]
fn membership_rejects_identity_drift_bad_schema_duplicate_fields_and_secret_reflections() {
    let mut cases = vec![
        json!({}),
        json!({"meta":{"code":200},"ctime":NOW,"data":null}),
    ];
    for (field, val) in [
        ("uid", json!(43)),
        ("uid", json!("042")),
        ("vipmExpire", json!(-1)),
        ("vipmExpire", json!(1.5)),
        ("vipmExpire", json!("01")),
        ("vipmExpire", json!(MAX_TIME + 1)),
        ("vipmAutoPayUser", json!(2)),
        ("isYearUser", json!(4294967296_u64)),
    ] {
        let mut value = body();
        value["data"][field] = val;
        cases.push(value);
    }
    for value in [json!(0), json!(-1), json!(MAX_TIME + 1), json!(true)] {
        let mut b = body();
        b["ctime"] = value;
        cases.push(b);
    }
    let mut b = body();
    b["meta"]["code"] = json!(500);
    cases.push(b);
    for value in cases {
        let e = parsed(&value).unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(!format!("{e:?}").contains(SID));
    }
    for raw in [
        r#"{"meta":{"code":200,"code":200},"ctime":1700000000000,"data":{"vipmExpire":0}}"#,
        r#"{"meta":{"code":200},"ctime":1700000000000,"data":{"vipmExpire":0,"vipmExpire":1700000000001}}"#,
    ] {
        assert!(parse(raw.as_bytes(), &input()).is_err());
    }
}

#[test]
fn invalid_optional_display_metadata_is_omitted_without_losing_membership_state() {
    for (field, value) in [
        ("vipTag", json!(SID)),
        ("userVipType", json!("x=private%2Dmembership%2Dsession")),
        ("vipTag", json!("x".repeat(1025))),
        ("vipIcon", json!("https://kuwo.cn.example.test/icon")),
        (
            "vipIcon",
            json!("https://img1.kuwo.cn/icon?sid=private-membership-session"),
        ),
    ] {
        let mut body = body();
        body["data"][field] = value;
        let summary = parsed(&body).unwrap();
        assert_eq!(summary.active, Some(true), "{field}");
        if field == "vipIcon" {
            assert!(summary.icon_url.is_none());
        } else if field == "vipTag" {
            assert!(summary.extensions["vip_tag"].is_null());
        } else {
            assert!(summary.extensions["user_vip_type"].is_null());
        }
        let serialized = serde_json::to_string(&summary).unwrap();
        assert!(!serialized.contains(SID));
    }
}

#[tokio::test]
async fn membership_sdk_validates_first_and_sends_bound_identity_metadata_without_cookie_rotation()
{
    let mut f = fixture::setup(vec![json_response(&json!({"result":"ok"})), reply()]).await;
    assert_eq!(
        f.client
            .native_membership(&credential())
            .await
            .unwrap()
            .active,
        Some(true)
    );
    let req = fixture::requests(&mut f, 2).await;
    assert!(req[0].starts_with("GET /u.s?"));
    let expected = format!(
        "GET {PATH}?op=ui&uid=42&sid={SID}&extend=1&showChezai=1&showLinqi=1&apiVersion=6&devid=1234567890&user=00112233445546778899aabbccddeeff&source=kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk&platform=ar HTTP/1.1\r\n"
    );
    assert!(req[1].starts_with(&expected));
    assert!(req[1].contains(&format!("loginUid=42,loginSid={SID},appUid=1234567890,")));
    assert!(!req[1].to_lowercase().contains("\r\ncookie:"));
    assert!(!req[1].contains("alien"));
}

#[tokio::test]
async fn membership_errors_never_retry_fall_back_or_return_a_partial_success() {
    let mut f = fixture::setup(vec![json_response(
        &json!({"result":"fail","reason":"error_user_invalid"}),
    )])
    .await;
    assert_eq!(
        f.client
            .native_membership(&credential())
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    fixture::requests(&mut f, 1).await;
    for b in [
        encrypted(&json!({"meta":{"code":500,"desc":SID},"ctime":NOW,"data":{}})),
        json_response(&body()),
        response(
            200,
            "text/html",
            "",
            b"<html>private-membership-session</html>",
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
            "Retry-After: 500\r\n",
            b"private-membership-session",
        ),
    ] {
        let mut f = fixture::setup(vec![json_response(&json!({"result":"ok"})), b]).await;
        let e = f.client.native_membership(&credential()).await.unwrap_err();
        assert!(!format!("{e:?}").contains(SID));
        fixture::requests(&mut f, 2).await;
    }
    let f = fixture::setup(vec![]).await;
    let injected = fixture::credential_fixture("42", "session,loginUid=43")
        .caller()
        .unwrap();
    assert_eq!(
        f.client
            .native_membership(&injected)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn membership_sdk_cancellation_and_timeouts_at_both_boundaries_return_no_data() {
    for boundary in 0..2 {
        for cancel in [false, true] {
            let gate = Arc::new(Notify::new());
            let mut f = fixture::setup_gated(
                [json_response(&json!({"result":"ok"})), reply()]
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect(),
            )
            .await;
            let client = f.client.clone();
            let task = tokio::spawn(async move { client.native_membership(&credential()).await });
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
