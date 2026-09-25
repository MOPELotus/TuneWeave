use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

const SID: &str = "collection-session&+%";
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", SID).caller().unwrap()
}
fn input() -> KuwoNativeSessionInput {
    credential::NativeCredential::parse(&credential())
        .unwrap()
        .input()
        .unwrap()
}
pub(crate) fn directory(count: Option<u64>) -> Value {
    let mut row = json!({"id":101,"name":"收藏歌单","desc":"literal + %20 &amp;"});
    if let Some(count) = count {
        row["total"] = json!(count);
    }
    json!({"result":"ok","data":[row]})
}
pub(crate) fn song(id: u64) -> Value {
    json!({"rid":id.to_string(),"name":format!("Song {id} + %20 &amp;"),"artist":"甲&乙",
        "artistid":"8","album":"Album","albumid":"9","img":"http://img3.kuwo.cn/star/albumcover/a.jpg",
        "duration":"123","content_type":"0","isstar":"0","digest":"15","ad_type":"1,5",
        "payInfo":"private-rights","token":"never-export","n_minfo":"MP3128|ALFLAC","isdownload":"0"})
}
pub(crate) fn page(ids: &[u64], pn: u64, total: u64) -> Value {
    json!({"code":"0","msg":"ok","data":{"total":total,"count":ids.len(),"pn":pn,"rn":100,
        "type":0,"sorttype":0,"musiclist":ids.iter().copied().map(song).collect::<Vec<_>>()}})
}
pub(crate) fn content(value: &Value) -> Vec<u8> {
    response(
        200,
        "text/plain; charset=utf-8",
        "Set-Cookie: ignored=never-export; Path=/\r\n",
        &serde_json::to_vec(value).unwrap(),
    )
}
pub(crate) fn flow(provider: bool) -> Vec<Vec<u8>> {
    let mut bodies = vec![json_response(&json!({"result":"ok"}))];
    if provider {
        bodies.push(json_response(&json!({"errcode":0,"plist":[]})));
    }
    bodies.push(json_response(&directory(Some(101))));
    for _ in 0..2 {
        bodies.push(content(&page(&(1..=100).collect::<Vec<_>>(), 0, 101)));
        bodies.push(content(&page(&[100], 1, 101)));
    }
    bodies.push(json_response(&directory(Some(101))));
    bodies
}
fn parse(value: &Value) -> Result<collected::Contents> {
    collected::parse(&serde_json::to_vec(value).unwrap(), &input(), "101", 0)
}

#[test]
fn collected_tracks_keep_metadata_duplicates_and_no_ad_or_entitlement_payload() {
    let tracks = parse(&page(&[11, 22, 22], 0, 3)).unwrap().tracks;
    assert_eq!(tracks[1], tracks[2]);
    let t = &tracks[0];
    assert_eq!(t.resource_ref.to_string(), "kuwo:11");
    assert_eq!(t.name, "Song 11 + %20 &amp;");
    assert_eq!(t.duration_ms, Some(123000));
    assert_eq!(t.artists[0].name, "甲&乙");
    assert!(t.artists[0].resource_ref.is_none());
    assert_eq!(
        t.album
            .as_ref()
            .unwrap()
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "9"
    );
    assert_eq!(
        t.album.as_ref().unwrap().cover_url.as_deref(),
        Some("http://img3.kuwo.cn/star/albumcover/a.jpg")
    );
    assert!(t.playable.is_none() && t.available_qualities.is_empty() && t.mv_ref.is_none());
    for secret in [
        SID,
        "private-rights",
        "never-export",
        "isdownload",
        "ad_type",
        "n_minfo",
    ] {
        assert!(!serde_json::to_string(&tracks).unwrap().contains(secret));
    }
    let mut minimal = page(&[11], 0, 1);
    minimal["data"]["musiclist"][0] = json!({"rid":"11","name":"Song"});
    let t = parse(&minimal).unwrap().tracks.remove(0);
    assert!(t.album.is_none() && t.artists.is_empty() && t.duration_ms.is_none());
}

#[test]
fn collected_parser_rejects_false_success_bad_page_identity_secrets_and_mixed_programs() {
    for (key, value) in [
        ("code", json!(1)),
        ("msg", json!("fail")),
        ("data", json!({})),
    ] {
        let mut bad = page(&[11], 0, 1);
        bad[key] = value;
        assert!(parse(&bad).is_err(), "{key}");
    }
    for (key, value) in [
        ("id", json!(102)),
        ("count", json!(0)),
        ("pn", json!(1)),
        ("rn", json!(20)),
        ("total", json!(0)),
        ("total", json!(10001)),
        ("type", json!(1)),
        ("sorttype", json!(1)),
        ("musiclist", json!(null)),
    ] {
        let mut bad = page(&[11], 0, 1);
        bad["data"][key] = value;
        assert!(parse(&bad).is_err(), "{key}");
    }
    for key in [
        "count",
        "total",
        "pn",
        "rn",
        "type",
        "sorttype",
        "musiclist",
    ] {
        let mut bad = page(&[11], 0, 1);
        bad["data"].as_object_mut().unwrap().remove(key);
        assert!(parse(&bad).is_err(), "{key}");
    }
    for (key, value) in [
        ("rid", json!("011")),
        ("rid", json!(0)),
        ("name", json!(SID)),
        ("name", json!("collection-session%26%2B%25")),
        ("artist", json!(SID)),
        ("album", json!(SID)),
        ("duration", json!(u64::MAX)),
        ("img", json!("https://evil.test/x")),
        ("name", json!("x".repeat(1025))),
        ("isstar", json!(1)),
        ("content_type", json!(2)),
        ("digest", json!(8)),
    ] {
        let mut bad = page(&[11, 22], 0, 2);
        bad["data"]["musiclist"][1][key] = value;
        assert!(parse(&bad).is_err(), "{key}");
    }
    assert!(collected::parse(br#"{"code":0,"code":1,"data":{}}"#, &input(), "101", 0).is_err());
    let raw = serde_json::to_string(&page(&[11], 0, 1))
        .unwrap()
        .replace("\"rid\":\"11\"", "\"rid\":\"11\",\"rid\":\"22\"");
    assert!(collected::parse(raw.as_bytes(), &input(), "101", 0).is_err());
    assert!(parse(&page(&[11], 0, 101)).is_err()); // truncated first page
    assert!(
        collected::parse(
            &serde_json::to_vec(&page(&[], 1, 1)).unwrap(),
            &input(),
            "101",
            1
        )
        .is_err()
    );
}

#[tokio::test]
async fn collected_sdk_binds_native_protocol_reads_twice_and_preserves_cross_page_duplicates() {
    let mut f = fixture::setup(flow(false)).await;
    let result = f
        .client
        .native_collected_playlist_tracks(&credential(), "101", &PageRequest::new(2, 99))
        .await
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["100", "100"]
    );
    assert_eq!(result.pagination.total, Some(101));
    assert!(!result.pagination.has_more);
    assert_eq!(
        result.pagination.extensions["backend"],
        "native_collected_playlist"
    );
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 6);
    let requests = fixture::requests(&mut f, 7).await;
    for (i, r) in requests.iter().enumerate().skip(1) {
        let target = r.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("https://fixture.test{target}")).unwrap();
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let q = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(pairs.len(), q.len());
        assert_eq!(q["loginUid"], "42");
        assert_eq!(q["loginSid"], SID);
        assert_eq!(q["appuid"], input().device_id());
        assert_eq!(q["user"], input().device_user());
        assert!(r.to_ascii_lowercase().contains("\r\ncookies: "));
        assert!(!r.to_ascii_lowercase().contains("\r\ncookie: "));
        if (2..=5).contains(&i) {
            assert_eq!(url.path(), "/list.s");
            assert_eq!(q["uid"], input().device_id());
            for (k, v) in [
                ("type", "songlist"),
                ("id", "101"),
                ("rn", "100"),
                ("sorttype", "0"),
                ("digest", "8"),
                ("aioper", "0"),
            ] {
                assert_eq!(q[k], v);
            }
            assert_eq!(q["pn"], ((i - 2) % 2).to_string());
            for k in ["isvip", "sid", "start", "count", "f"] {
                assert!(!q.contains_key(k));
            }
        } else {
            assert_eq!(q["type"], "get_like_sl");
            assert_eq!(q["uid"], "42");
        }
    }
    let mut f = fixture::setup(flow(false)).await;
    let metadata = f
        .client
        .native_collected_playlist(&credential(), "101")
        .await
        .unwrap();
    assert_eq!(
        metadata.extensions["source_snapshot_id"],
        result.pagination.extensions["source_snapshot_id"]
    );
    assert_eq!(metadata.subscribed, Some(true));
    assert!(metadata.creator.is_none());
    assert!(!metadata.extensions.contains_key("owner_id"));
    fixture::requests(&mut f, 7).await;
}

#[tokio::test]
async fn collected_lookup_completes_more_than_one_hundred_directory_entries() {
    let directories=(0..6).map(|pn| json!({"result":"ok","data":(0..20).map(|i|json!({"id":1000+pn*20+i,"name":"Other","total":0})).collect::<Vec<_>>()})).chain([directory(Some(101))]).collect::<Vec<_>>();
    let mut bodies = vec![json_response(&json!({"result":"ok"}))];
    bodies.extend(directories.iter().map(json_response));
    bodies.extend(flow(false).into_iter().skip(2).take(4));
    bodies.extend(directories.iter().map(json_response));
    let mut f = fixture::setup(bodies).await;
    let metadata = f
        .client
        .native_collected_playlist(&credential(), "101")
        .await
        .unwrap();
    assert_eq!(metadata.track_count, Some(101));
    assert_eq!(metadata.extensions["upstream_pages_fetched"], 18);
    let r = fixture::requests(&mut f, 19).await;
    assert!(r[7].contains("start=120"));
    assert!(r[18].contains("start=120"));
}

#[tokio::test]
async fn collected_empty_out_of_range_and_missing_membership_have_explicit_boundaries() {
    let empty = content(&page(&[], 0, 0));
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(Some(0))),
        empty.clone(),
        empty.clone(),
        json_response(&directory(Some(0))),
    ])
    .await;
    let result = f
        .client
        .native_collected_playlist_tracks(&credential(), "101", &PageRequest::new(2, 100))
        .await
        .unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.pagination.total, Some(0));
    fixture::requests(&mut f, 5).await;
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(None)),
        empty,
    ])
    .await;
    assert!(
        f.client
            .native_collected_playlist(&credential(), "101")
            .await
            .is_err()
    );
    fixture::requests(&mut f, 3).await;
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&json!({"result":"ok","data":[]})),
    ])
    .await;
    assert_eq!(
        f.client
            .native_collected_playlist(&credential(), "101")
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    fixture::requests(&mut f, 2).await;
    let mut f = fixture::setup(flow(false)).await;
    let result = f
        .client
        .native_collected_playlist_tracks(&credential(), "101", &PageRequest::new(2, 500))
        .await
        .unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.pagination.total, Some(101));
    fixture::requests(&mut f, 7).await;
}

#[tokio::test]
async fn collected_changes_failures_and_mime_budget_never_yield_partial_success() {
    let mut renamed = directory(Some(101));
    renamed["data"][0]["name"] = json!("Changed");
    let mut edits = vec![
        (0, json_response(&json!({"result":"fail"})), 1),
        (1, json_response(&directory(Some(10001))), 2),
        (3, content(&page(&[100], 1, 102)), 4),
        (5, content(&page(&[99], 1, 101)), 6),
        (6, json_response(&renamed), 7),
        (6, json_response(&json!({"result":"ok","data":[]})), 7),
        (3, response(200, "text/html", "", b"<html>login</html>"), 4),
        (3, content(&json!({"code":1,"msg":SID})), 4),
        (
            2,
            response(
                200,
                "text/plain",
                "",
                &vec![b' '; library::MAX_RESPONSE + 1],
            ),
            3,
        ),
    ];
    for status in [302, 401, 403, 429, 500] {
        edits.push((3, response(status, "text/plain", "", b"private"), 4));
    }
    for (at, bad, count) in edits {
        let mut bodies = flow(false);
        bodies[at] = bad;
        bodies.truncate(count);
        let mut f = fixture::setup(bodies).await;
        let e = f
            .client
            .native_collected_playlist(&credential(), "101")
            .await
            .unwrap_err();
        assert!(!e.to_string().contains(SID));
        fixture::requests(&mut f, count).await;
    }
    // text/plain is accepted only for the fixed collected-content path.
    let mut f = fixture::setup(vec![response(200, "text/plain", "", br#"{"result":"ok"}"#)]).await;
    assert!(
        f.client
            .native_collected_playlist(&credential(), "101")
            .await
            .is_err()
    );
    fixture::requests(&mut f, 1).await;
}

#[tokio::test]
async fn collected_invalid_inputs_never_reach_the_network() {
    let mut f = fixture::setup(vec![]).await;
    for id in ["0", "0101", "-1", "101&sid=x", "9223372036854775808"] {
        assert_eq!(
            f.client
                .native_collected_playlist(&credential(), id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert_eq!(
            f.client
                .native_collected_playlist_tracks(
                    &credential(),
                    "101",
                    &PageRequest::new(limit, offset)
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut r = PageRequest::new(1, 0);
    r.account = Some("stored".into());
    assert_eq!(
        f.client
            .native_collected_playlist_tracks(&credential(), "101", &r)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    fixture::requests(&mut f, 0).await;
}
