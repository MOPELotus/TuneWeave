use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

pub(crate) fn track() -> Track {
    Track::new(kuwo_track_ref("67474").unwrap(), "Synthetic media")
}
pub(crate) fn request(account: Option<&str>) -> StreamRequest {
    StreamRequest {
        quality: Quality::Standard,
        account: account.map(str::to_owned),
        ..StreamRequest::default()
    }
}
pub(crate) fn rights() -> Value {
    json!({"result":"ok","errorcode":0,"timestamp":1789540000,"songs":[{
        "id":67474,"token":{"ZPLY":"bbbbbbbbbbbbbbbbcccccccccccccccc","H":"11111111111111112222222222222222","S":"33333333333333334444444444444444","F":"55555555555555556666666666666666","HR":"9999999999999999aaaaaaaaaaaaaaaa","L":"77777777777777778888888888888888"},
        "audio":[{"quality":"H","br":128,"fmt":"MP3128","policy":"","st":0,"price":0,"avaliable":1}],
        "payInfo":{"nplay":"000111111111","ndown":"000111111111","cannotOnlinePlay":0,"cannotDownload":0},
        "unknown_secret":"do-not-export"
    }]})
}
pub(crate) fn media() -> Value {
    json!({"code":200,"duration":240,"data":{"rid":67474,"format":"mp3","bitrate":128,"quality":"H","url":"http://er-sycdn.kuwo.cn/token/time/file.mp3","surl":"https://er-sycdn.kuwo.cn/token/time/file.mp3","type":0,"startPos":0,"endPos":0,"ekey":"","sig":"12345","unknown_secret":"do-not-export"}})
}
pub(crate) fn rights_reply(value: &Value) -> Vec<u8> {
    response(
        200,
        "text/plain; charset=utf-8",
        "Set-Cookie: sid=do-not-adopt; Path=/\r\n",
        &serde_json::to_vec(value).unwrap(),
    )
}
pub(crate) fn flow() -> Vec<Vec<u8>> {
    vec![
        json_response(&json!({"result":"ok"})),
        rights_reply(&rights()),
        json_response(&media()),
    ]
}
pub(crate) fn query(request: &str) -> BTreeMap<String, String> {
    let url = Url::parse(&format!(
        "https://fixture.invalid{}",
        request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
    ))
    .unwrap();
    if url.path() != MEDIA_PATH {
        return url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
    }
    use aes::{
        Aes128,
        cipher::{BlockDecrypt, KeyInit},
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE};
    let q = url
        .query_pairs()
        .find(|(k, _)| k == "q")
        .unwrap()
        .1
        .replace('.', "=");
    let mut data = URL_SAFE.decode(q).unwrap();
    // Independent fixed vector from the pinned native implementation.
    let cipher = Aes128::new_from_slice(b"14505b09374869d9").unwrap();
    for block in data.chunks_exact_mut(16) {
        cipher.decrypt_block(block.into());
    }
    let padding = usize::from(*data.last().unwrap());
    assert!((1..=16).contains(&padding));
    assert!(
        data[data.len() - padding..]
            .iter()
            .all(|v| usize::from(*v) == padding)
    );
    data.truncate(data.len() - padding);
    url::form_urlencoded::parse(&data)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[tokio::test]
async fn native_media_sdk_checks_action_identity_quality_and_actual_delivery() {
    for (spec, quality) in [
        (LOW, Quality::Low),
        (STANDARD, Quality::Standard),
        (HIGH, Quality::Higher),
        (LOSSLESS, Quality::Lossless),
        (HI_RES, Quality::Hires),
    ] {
        for action in [Action::Play, Action::Download] {
            let credential = fixture::credential_fixture("42", "selected&+%session")
                .caller()
                .unwrap();
            let mut rights = rights();
            rights["songs"][0]["audio"][0] = json!({"quality":spec.tag,"br":spec.selector,"fmt":spec.rights_format,"policy":"vip","st":0,"cost":0,"price":5});
            let mut media = media();
            let data = &mut media["data"];
            data["format"] = json!(spec.format);
            data["quality"] = json!(spec.tag);
            data["bitrate"] = json!(spec.selector);
            data["url"] = json!(format!(
                "http://er-sycdn.kuwo.cn/token/file.{}",
                spec.format
            ));
            data["surl"] = json!(format!(
                "https://er-sycdn.kuwo.cn/token/file.{}",
                spec.format
            ));
            let mut f = fixture::setup(vec![
                flow().remove(0),
                rights_reply(&rights),
                json_response(&media),
            ])
            .await;
            let req = StreamRequest {
                quality,
                ..request(None)
            };
            let shown = match action {
                Action::Play => {
                    let result = f
                        .client
                        .native_stream(&credential, &track(), &req)
                        .await
                        .unwrap();
                    assert_eq!(result.actual_quality, spec.quality);
                    assert_eq!(result.bitrate, spec.bitrate);
                    assert_eq!(result.duration_ms, Some(240_000));
                    assert!(result.trial.is_none());
                    serde_json::to_string(&result).unwrap()
                }
                Action::Download => {
                    let result = f
                        .client
                        .native_download(&credential, &track(), &req)
                        .await
                        .unwrap();
                    assert!(result.available);
                    assert_eq!(result.bitrate, spec.bitrate);
                    assert_eq!(result.format.as_deref(), Some(spec.format));
                    serde_json::to_string(&result).unwrap()
                }
            };
            for hidden in [
                "selected&+%session",
                "do-not-export",
                "do-not-adopt",
                "11111111111111112222222222222222",
            ] {
                assert!(!shown.contains(hidden));
            }
            let calls = fixture::requests(&mut f, 3).await;
            let rights_query = query(&calls[1]);
            let media_query = query(&calls[2]);
            assert_eq!(rights_query["action"], action.rights());
            assert_eq!(rights_query["uid"], "42");
            assert_eq!(rights_query["sid"], "selected&+%session");
            assert_eq!(rights_query["quality"], spec.tag);
            assert_eq!(media_query["loginUid"], "42");
            assert_eq!(media_query["loginSid"], "selected&+%session");
            assert_eq!(media_query["payUid"], "42");
            assert_eq!(media_query["uid"], media_query["appuid"]);
            assert_ne!(media_query["uid"], "42");
            assert_eq!(media_query["mode"], action.mode());
            assert_eq!(
                media_query["br"],
                format!("{}k{}", spec.selector, spec.format)
            );
            assert_eq!(media_query["format"], spec.format);
            assert_eq!(media_query["kpk"], "a26692b114b983a72a19db4d6ce652bf");
            assert_eq!(media_query["timestamp"], "1789540000");
            assert_eq!(media_query["bc_token"], "");
            assert!(
                calls
                    .iter()
                    .all(|r| !r.to_ascii_lowercase().contains("cookie:"))
            );
        }
    }
}

#[tokio::test]
async fn native_media_tokens_do_not_grant_playback_or_download_and_auto_uses_proven_rights() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for st in [102, 103, 104, 107, 201, 502, 1000] {
        let mut rights = rights();
        rights["songs"][0]["audio"][0]["policy"] = json!("vip");
        rights["songs"][0]["audio"][0]["st"] = json!(st);
        let mut f = fixture::setup(vec![flow().remove(0), rights_reply(&rights)]).await;
        let result = f
            .client
            .native_download(&credential, &track(), &request(None))
            .await
            .unwrap();
        assert!(!result.available);
        assert!(result.url.is_none());
        fixture::requests(&mut f, 2).await;
    }
    let mut rights = rights();
    rights["songs"][0]["audio"]
        .as_array_mut()
        .unwrap()
        .push(json!({"quality":"F","br":2000,"fmt":"ALFLAC","policy":"vip","st":102}));
    let mut f = fixture::setup(vec![
        flow().remove(0),
        rights_reply(&rights),
        json_response(&media()),
    ])
    .await;
    let stream = f
        .client
        .native_stream(&credential, &track(), &StreamRequest::default())
        .await
        .unwrap();
    assert_eq!(stream.actual_quality, Quality::Standard);
    let calls = fixture::requests(&mut f, 3).await;
    assert_eq!(
        query(&calls[2])["token"],
        "11111111111111112222222222222222"
    );
}

#[tokio::test]
async fn native_media_rights_require_bound_explicit_and_bounded_fields() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let mut bodies = Vec::new();
    for (pointer, value) in [
        ("/result", json!("fail")),
        ("/errorcode", json!(1)),
        ("/uid", json!("43")),
        ("/timestamp", json!(0)),
        ("/songs/0/id", json!(43)),
        ("/songs/0/audio/0/st", json!(null)),
        ("/songs/0/audio/0/st", json!(99999)),
        ("/songs/0/audio/0/policy", json!("unknown")),
        ("/songs/0/audio/0/br", json!(128.0)),
        ("/songs/0/payInfo/nplay", json!("0&uid=43")),
        ("/songs/0/token/H", json!("selected-session")),
        ("/songs/0/token/H", json!("")),
        ("/songs/0/token/H", json!("line\nbreak")),
        ("/songs/0/token", json!("{}")),
    ] {
        let mut body = rights();
        if pointer == "/uid" {
            body["uid"] = value;
        } else {
            *body.pointer_mut(pointer).unwrap() = value;
        }
        bodies.push(body);
    }
    let mut duplicate = rights();
    let song = duplicate["songs"][0].clone();
    duplicate["songs"].as_array_mut().unwrap().push(song);
    bodies.push(duplicate);
    let mut empty = rights();
    empty["songs"] = json!([]);
    bodies.push(empty);
    let mut large = rights();
    large["songs"][0]["audio"] = json!(vec![large["songs"][0]["audio"][0].clone(); 129]);
    bodies.push(large);
    for body in bodies {
        let mut f = fixture::setup(vec![flow().remove(0), rights_reply(&body)]).await;
        let error = f
            .client
            .native_stream(&credential, &track(), &request(None))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.to_string().contains("selected-session"));
        fixture::requests(&mut f, 2).await;
    }
    for policy in ["vip", ""] {
        let mut body = rights();
        body["songs"][0]["audio"][0]["policy"] = json!(policy);
        body["songs"][0]["audio"][0]
            .as_object_mut()
            .unwrap()
            .remove("price");
        let mut f = fixture::setup(vec![flow().remove(0), rights_reply(&body)]).await;
        assert_eq!(
            f.client
                .native_stream(&credential, &track(), &request(None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        fixture::requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn native_media_play_and_download_rights_are_never_reused() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let mut denied = rights();
    denied["songs"][0]["payInfo"]["cannotDownload"] = json!(1);
    let mut bodies = flow();
    bodies.extend([flow().remove(0), rights_reply(&denied)]);
    let mut f = fixture::setup(bodies).await;
    assert!(
        f.client
            .native_stream(&credential, &track(), &request(None))
            .await
            .is_ok()
    );
    assert!(
        !f.client
            .native_download(&credential, &track(), &request(None))
            .await
            .unwrap()
            .available
    );
    let calls = fixture::requests(&mut f, 5).await;
    assert_eq!(query(&calls[1])["action"], "play");
    assert_eq!(query(&calls[4])["action"], "download");
}

#[tokio::test]
async fn native_media_response_never_turns_trials_or_encrypted_files_into_full_audio() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for (pointer, value, expected) in [
        ("/data/type", json!(1), ErrorCode::PermissionDenied),
        ("/data/startPos", json!(10), ErrorCode::PermissionDenied),
        ("/data/endPos", json!(30), ErrorCode::PermissionDenied),
        (
            "/data/ekey",
            json!("encrypted-key"),
            ErrorCode::UpstreamError,
        ),
    ] {
        let mut body = media();
        *body.pointer_mut(pointer).unwrap() = value;
        let mut f = fixture::setup(vec![
            flow().remove(0),
            rights_reply(&rights()),
            json_response(&body),
        ])
        .await;
        assert_eq!(
            f.client
                .native_stream(&credential, &track(), &request(None))
                .await
                .unwrap_err()
                .code,
            expected
        );
        fixture::requests(&mut f, 3).await;
    }
}

#[tokio::test]
async fn native_media_response_rejects_mismatched_shapes_hosts_and_unknown_successes() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let mut bodies = Vec::new();
    for (pointer, value) in [
        ("/code", json!(0)),
        ("/duration", json!(0)),
        ("/duration", json!(86_401)),
        ("/data/rid", json!(42)),
        ("/data/bitrate", json!(320)),
        ("/data/format", json!("mgg")),
        ("/data/quality", json!("S")),
        ("/data/type", json!(null)),
        ("/data/startPos", json!(-1)),
        ("/data/endPos", json!(null)),
    ] {
        let mut body = media();
        *body.pointer_mut(pointer).unwrap() = value;
        bodies.push(body);
    }
    for url in [
        "https://evil.example/file.mp3",
        "https://er-sycdn.kuwo.cn.evil.example/file.mp3",
        "https://nested.er-sycdn.kuwo.cn/file.mp3",
        "https://user@er-sycdn.kuwo.cn/file.mp3",
        "https://er-sycdn.kuwo.cn:444/file.mp3",
        "https://er-sycdn.kuwo.cn/file.mgg",
        "https://er-sycdn.kuwo.cn/file.mp3?sid=selected-session",
        "https://er-sycdn.kuwo.cn/selected-session/file.mp3",
        "https://er-sycdn.kuwo.cn/%73elected-session/file.mp3",
        "https://er-sycdn.kuwo.cn/file.mp3#x",
        "file:///tmp/file.mp3",
    ] {
        let mut body = media();
        body["data"]["surl"] = json!(url);
        bodies.push(body);
    }
    let mut http = media();
    http["data"]["surl"] = json!("");
    bodies.push(http);
    let mut other_sid = media();
    other_sid["loginSid"] = json!("other-session");
    bodies.push(other_sid);
    for body in bodies {
        let mut f = fixture::setup(vec![
            flow().remove(0),
            rights_reply(&rights()),
            json_response(&body),
        ])
        .await;
        assert_eq!(
            f.client
                .native_stream(&credential, &track(), &request(None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        fixture::requests(&mut f, 3).await;
    }
}

#[tokio::test]
async fn native_media_business_failures_are_distinct_from_stale_sid_and_never_echo_raw_messages() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for code in [
        401, 402, 403, 404, 407, 4012, 4015, 4017, 4018, 5001, 5002, 9999,
    ] {
        let body =
            json!({"code":code,"loginSid":"selected-session","msg":"selected-session + raw-token"});
        let mut f = fixture::setup(vec![
            flow().remove(0),
            rights_reply(&rights()),
            json_response(&body),
        ])
        .await;
        let error = f
            .client
            .native_stream(&credential, &track(), &request(None))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            match code {
                4015 | 4017 => ErrorCode::AuthenticationRequired,
                9999 => ErrorCode::UpstreamError,
                _ => ErrorCode::PermissionDenied,
            }
        );
        assert!(!format!("{error:?}").contains("selected-session"));
        fixture::requests(&mut f, 3).await;
    }
}

#[tokio::test]
async fn native_media_transport_bounds_mime_redirects_and_early_request_errors() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for reply in [
        response(
            302,
            "application/json",
            "Location: https://evil.example/\r\n",
            b"",
        ),
        response(200, "text/html", "", b"{}"),
        response(200, "text/plain", "", &vec![b'x'; MAX_RESPONSE + 1]),
        response(401, "application/json", "", b"private"),
    ] {
        let mut f = fixture::setup(vec![flow().remove(0), reply]).await;
        assert!(
            f.client
                .native_stream(&credential, &track(), &request(None))
                .await
                .is_err()
        );
        fixture::requests(&mut f, 2).await;
    }
    let mut f = fixture::setup(vec![]).await;
    for req in [
        StreamRequest {
            quality: Quality::Hires,
            ..request(None)
        },
        StreamRequest {
            bitrate: Some(192_000),
            ..request(None)
        },
        StreamRequest {
            bitrate: Some(0),
            ..request(None)
        },
    ] {
        assert!(
            f.client
                .native_stream(&credential, &track(), &req)
                .await
                .is_err()
        );
    }
    let mut wrong = track();
    wrong.resource_ref = ResourceRef::new(Platform::Kugou, "67474").unwrap();
    assert!(
        f.client
            .native_stream(&credential, &wrong, &request(None))
            .await
            .is_err()
    );
    fixture::requests(&mut f, 0).await;
}

#[tokio::test]
async fn native_media_availability_verifies_media_and_reports_actual_bitrate_without_urls() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let mut f = fixture::setup(flow()).await;
    let result = f
        .client
        .native_track_availability(&credential, "67474", &TrackAvailabilityRequest::default())
        .await
        .unwrap();
    assert!(result.playable);
    assert_eq!(result.actual_bitrate, Some(128_000));
    let shown = serde_json::to_string(&result).unwrap();
    assert!(!shown.contains("https://"));
    assert!(!shown.contains("token"));
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_media_policy_groups_accept_purchases_but_duplicate_tokens_are_invalid() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for policy in ["song", "album"] {
        let mut r = rights();
        r["songs"][0]["audio"] = json!([
            {"quality":"H","br":128,"fmt":"MP3128","policy":"vip","st":102,"cost":5},
            {"quality":"H","br":128,"fmt":"MP3128","policy":policy,"st":0}
        ]);
        let mut f = fixture::setup(vec![
            flow().remove(0),
            rights_reply(&r),
            json_response(&media()),
        ])
        .await;
        assert!(
            f.client
                .native_stream(&credential, &track(), &request(None))
                .await
                .is_ok()
        );
        fixture::requests(&mut f, 3).await;
    }
    let encoded = rights()
        .to_string()
        .replace("\"token\":{", "\"token\":{\"H\":\"duplicate\",");
    let mut f = fixture::setup(vec![
        flow().remove(0),
        response(200, "text/plain", "", encoded.as_bytes()),
    ])
    .await;
    assert_eq!(
        f.client
            .native_stream(&credential, &track(), &request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    fixture::requests(&mut f, 2).await;
}
