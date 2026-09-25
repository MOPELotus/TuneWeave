use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;
use tokio::sync::Notify;

const SID: &str = "playlist-session&+%";
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
    let mut row =
        json!({"type":"GENERAL","id":101,"title":"自己的歌单","info":"原始 + %20","ispub":false});
    if let Some(n) = count {
        row["musicnum"] = json!(n);
    }
    json!({"errcode":0,"result":"ok","plist":[row]})
}
pub(crate) fn song(id: u64) -> Value {
    json!({"id":id,"name":format!("歌曲 {id} + %20 &amp;"),"artist":"歌手甲&歌手乙","artistid":9,
        "album":"专辑 + %20","albumid":88,"duration":"123","albumpic":"https://img4.kuwo.cn/star/albumcover/a.jpg",
        "content_type":0,"isstar":"0","payInfo":"private-rights","token":"never-export","N_MINFO":"never-export","isdownload":0,"formats":"MP3128|ALFLAC"})
}
pub(crate) fn page(ids: &[u64], pages: u64) -> Value {
    json!({"errcode":0,"result":"ok","songchange":true,"pagenum":pages,
        "info":{"musiclist":ids.iter().copied().map(song).collect::<Vec<_>>()}})
}
pub(crate) fn detail(uid: &str, count: u64) -> Value {
    json!({"sl_data":{"uid":uid,"title":"自己的歌单","desc":"原始 + %20","tag":"流行,安静","tagid":"393,400",
        "pic":"","big_pic":"","total":count,"igsl":"0","token":"never-export"}})
}
pub(crate) fn flow() -> Vec<Vec<u8>> {
    flow_for("42")
}
pub(crate) fn flow_for(uid: &str) -> Vec<Vec<u8>> {
    [
        json!({"result":"ok"}),
        directory(Some(4)),
        detail(uid, 4),
        page(&[11, 22], 2),
        page(&[22, 33], 2),
        page(&[11, 22], 2),
        page(&[22, 33], 2),
        detail(uid, 4),
        directory(Some(4)),
    ]
    .iter()
    .map(json_response)
    .collect()
}
fn parse(value: &Value, empty: bool) -> Result<dto::Contents> {
    dto::parse(&serde_json::to_vec(value).unwrap(), &input(), "101", empty)
}

#[test]
fn native_playlist_tracks_preserve_literal_metadata_and_never_create_entitlements() {
    let tracks = parse(&page(&[11, 22, 22], 1), false).unwrap().tracks;
    assert_eq!(tracks[1], tracks[2]);
    let t = &tracks[0];
    assert_eq!(t.resource_ref.to_string(), "kuwo:11");
    assert_eq!(t.name, "歌曲 11 + %20 &amp;");
    assert_eq!(t.duration_ms, Some(123000));
    assert_eq!(t.artists[0].name, "歌手甲&歌手乙");
    assert!(t.artists[0].resource_ref.is_none());
    assert_eq!(
        t.album
            .as_ref()
            .unwrap()
            .resource_ref
            .as_ref()
            .unwrap()
            .to_string(),
        "kuwo:88"
    );
    assert!(t.playable.is_none() && t.available_qualities.is_empty() && t.mv_ref.is_none());
    let serialized = serde_json::to_string(&tracks).unwrap();
    for secret in [
        SID,
        "private-rights",
        "never-export",
        "isdownload",
        "N_MINFO",
    ] {
        assert!(!serialized.contains(secret));
    }
    let mut minimal = page(&[11], 1);
    minimal["info"]["musiclist"][0] = json!({"id":11,"name":"Song"});
    let track = parse(&minimal, false).unwrap().tracks.remove(0);
    assert!(track.artists.is_empty() && track.album.is_none() && track.duration_ms.is_none());
    let mut solo = page(&[11], 1);
    solo["info"]["musiclist"][0]["artist"] = json!("Singer");
    assert_eq!(
        parse(&solo, false).unwrap().tracks[0].artists[0]
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "9"
    );
}

#[test]
fn native_playlist_parser_rejects_ambiguous_empty_identity_secrets_and_nonmusic() {
    for bad in [
        json!({}),
        json!({"errcode":0,"songchange":false}),
        json!({"errcode":0,"songchange":true,"pagenum":1}),
        json!({"errcode":400,"songchange":0}),
        json!({"errcode":0,"songchange":true,"pagenum":16,"info":{"musiclist":[]}}),
    ] {
        assert!(parse(&bad, false).is_err());
    }
    for (field, value) in [
        ("errcode", json!(401)),
        ("result", json!("fail")),
        ("uid", json!(43)),
        ("pid", json!(102)),
        ("pagenum", json!(0)),
        ("songchange", json!(false)),
        ("songchange", json!(1)),
    ] {
        let mut bad = page(&[11], 1);
        bad[field] = value;
        assert!(parse(&bad, false).is_err(), "{field}");
    }
    for (field, value) in [
        ("id", json!(0)),
        ("id", json!("011")),
        ("name", json!(SID)),
        ("name", json!("playlist-session%26%2B%25")),
        ("artist", json!(SID)),
        ("album", json!(SID)),
        ("name", json!("\u{0}")),
        ("name", json!("x".repeat(1025))),
        ("duration", json!(u64::MAX)),
        ("albumpic", json!("https://evil.test/a")),
        ("isstar", json!(1)),
        ("content_type", json!(2)),
    ] {
        let mut bad = page(&[11], 1);
        bad["info"]["musiclist"][0][field] = value;
        assert!(parse(&bad, false).is_err(), "{field}");
    }
    for raw in [
        r#"{"errcode":0,"errcode":1,"songchange":false}"#,
        r#"{"errcode":0,"songchange":true,"pagenum":1,"info":{"musiclist":[{"id":1,"id":2,"name":"x"}]}}"#,
    ] {
        assert!(dto::parse(raw.as_bytes(), &input(), "101", true).is_err());
    }
    let empty = json!({"errcode":0,"songchange":false});
    assert!(parse(&empty, true).unwrap().tracks.is_empty());
    assert!(parse(&empty, false).is_err());
    let mut false_nonempty = page(&[11], 1);
    false_nonempty["songchange"] = json!(false);
    assert!(parse(&false_nonempty, true).is_err());
}

#[tokio::test]
async fn native_playlist_sdk_reads_both_complete_traversals_before_window_and_binds_protocol() {
    let mut f = fixture::setup(flow()).await;
    let result = f
        .client
        .native_created_playlist_tracks(&credential(), "101", &PageRequest::new(2, 1))
        .await
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["22", "22"]
    );
    assert_eq!(result.pagination.total, Some(4));
    assert_eq!(result.pagination.next_offset, Some(3));
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 8);
    let digest = result.pagination.extensions["source_snapshot_id"].clone();
    let requests = fixture::requests(&mut f, 9).await;
    for (i, request) in requests.iter().enumerate().skip(1) {
        let target = request.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("https://fixture.test{target}")).unwrap();
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let q = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(q.len(), pairs.len());
        assert_eq!(q["uid"], "42");
        if [2, 7].contains(&i) {
            assert_eq!(url.path(), metadata::METADATA_PATH);
            assert_eq!(q["type"], "get_songlist_info2");
            assert_eq!(q["id"], "101");
            assert_eq!(q["loginUid"], "42");
            assert_eq!(q["loginSid"], SID);
            assert!(!q.contains_key("op"));
            continue;
        }
        assert_eq!(q["sid"], SID);
        assert_eq!(q["devid"], input().device_id());
        assert_eq!(q["user"], input().device_user());
        assert!(request.to_ascii_lowercase().contains("\r\ncookies: "));
        assert!(!request.to_ascii_lowercase().contains("\r\ncookie: "));
        if (3..=6).contains(&i) {
            assert_eq!(q["op"], "pl3_getlist");
            assert_eq!(q["pid"], "101");
            assert_eq!(q["sig"], "0");
            assert_eq!(q["pn"], ((i - 3) % 2).to_string());
            assert!(!q.contains_key("recommend"));
        } else {
            assert_eq!(q["op"], "pl3_getuserlists");
        }
    }
    let mut f = fixture::setup(flow()).await;
    let metadata = f
        .client
        .native_created_playlist(&credential(), "101")
        .await
        .unwrap();
    assert_eq!(metadata.extensions["source_snapshot_id"], digest);
    assert_eq!(metadata.track_count, Some(4));
    assert_eq!(metadata.tags, ["流行", "安静"]);
    assert_eq!(metadata.extensions["editable_metadata_verified"], true);
    fixture::requests(&mut f, 9).await;
}

#[tokio::test]
async fn native_playlist_sdk_empty_and_out_of_range_still_require_complete_confirmation() {
    for empty in [
        json!({"errcode":0,"songchange":0}),
        page(&[], 1),
        json!({"errcode":0,"songchange":false,"pagenum":0,"info":{"musiclist":[]}}),
    ] {
        let replies = [
            json!({"result":"ok"}),
            directory(Some(0)),
            detail("42", 0),
            empty.clone(),
            empty,
            detail("42", 0),
            directory(Some(0)),
        ];
        let mut f = fixture::setup(replies.iter().map(json_response).collect()).await;
        let result = f
            .client
            .native_created_playlist_tracks(&credential(), "101", &PageRequest::new(100, 50))
            .await
            .unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.pagination.total, Some(0));
        assert!(!result.pagination.has_more);
        fixture::requests(&mut f, 7).await;
    }
    let mut f = fixture::setup(flow()).await;
    let result = f
        .client
        .native_created_playlist_tracks(&credential(), "101", &PageRequest::new(10, 100))
        .await
        .unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.pagination.total, Some(4));
    fixture::requests(&mut f, 9).await;
}

#[tokio::test]
async fn native_playlist_mid_read_failures_and_changes_never_return_partial_or_cached_success() {
    let mut changes = Vec::new();
    changes.push((0, json_response(&json!({"result":"fail"}))));
    changes.push((1, json_response(&json!({"errcode":0,"plist":[]}))));
    for (index, value) in [
        (3, page(&[], 2)),
        (4, page(&[22, 33], 3)),
        (5, page(&[11, 99], 2)),
        (8, directory(Some(5))),
    ] {
        changes.push((index, json_response(&value)));
    }
    let mut renamed = directory(Some(4));
    renamed["plist"][0]["title"] = json!("Changed");
    changes.push((8, json_response(&renamed)));
    for status in [302, 401, 403, 429, 500] {
        changes.push((
            4,
            response(status, "application/json", "", b"private upstream response"),
        ));
    }
    changes.push((4, response(200, "text/html", "", b"<html>login</html>")));
    changes.push((4, json_response(&json!({"errcode":603,"reason":SID}))));
    changes.push((
        3,
        response(
            200,
            "application/json",
            "",
            &vec![b' '; library::MAX_RESPONSE + 1],
        ),
    ));
    for (index, bad) in changes {
        let mut replies = flow();
        replies[index] = bad;
        // Pass two's content mismatch is discovered after finishing pass two.
        let count = if index == 5 { 7 } else { index + 1 };
        replies.truncate(count);
        let mut f = fixture::setup(replies).await;
        let error = f
            .client
            .native_created_playlist_tracks(&credential(), "101", &PageRequest::new(1, 0))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains(SID));
        fixture::requests(&mut f, count).await;
    }
}

#[tokio::test]
async fn native_playlist_preflight_and_limits_never_request_unbounded_reads() {
    let mut f = fixture::setup(vec![]).await;
    for id in ["0", "0101", "-1", "101&uid=7", "9223372036854775808"] {
        assert_eq!(
            f.client
                .native_created_playlist(&credential(), id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert_eq!(
            f.client
                .native_created_playlist_tracks(
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
    let mut request = PageRequest::new(1, 0);
    request.account = Some("stored".into());
    assert!(
        f.client
            .native_created_playlist_tracks(&credential(), "101", &request)
            .await
            .is_err()
    );
    fixture::requests(&mut f, 0).await;
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(Some(MAX_TRACKS as u64 + 1))),
    ])
    .await;
    assert!(
        f.client
            .native_created_playlist(&credential(), "101")
            .await
            .is_err()
    );
    fixture::requests(&mut f, 2).await;
    let mut replies = vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(None)),
        json_response(&detail("42", MAX_TRACKS as u64)),
    ];
    let items = vec![11; 1000];
    for _ in 0..11 {
        replies.push(json_response(&page(&items, 15)));
    }
    let mut f = fixture::setup(replies).await;
    assert!(
        f.client
            .native_created_playlist(&credential(), "101")
            .await
            .is_err()
    );
    fixture::requests(&mut f, 14).await;
    let mut replies = flow();
    replies[3] = json_response(&page(&[11], 16));
    replies.truncate(4);
    let mut f = fixture::setup(replies).await;
    assert!(
        f.client
            .native_created_playlist(&credential(), "101")
            .await
            .is_err()
    );
    fixture::requests(&mut f, 4).await;
}

#[tokio::test]
async fn native_playlist_sdk_cancellation_at_each_boundary_discards_the_incomplete_read() {
    for boundary in 0..flow().len() {
        let gate = Arc::new(Notify::new());
        let mut f = fixture::setup_gated(
            flow()
                .into_iter()
                .enumerate()
                .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                .collect(),
        )
        .await;
        let client = f.client.clone();
        let task =
            tokio::spawn(async move { client.native_created_playlist(&credential(), "101").await });
        for _ in 0..=boundary {
            tokio::time::timeout(Duration::from_secs(3), f.seen.recv())
                .await
                .unwrap()
                .unwrap();
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(f.seen.try_recv().is_err());
    }
}
