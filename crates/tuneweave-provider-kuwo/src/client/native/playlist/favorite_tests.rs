use super::tests::page;
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;

const SID: &str = "favorite-session&+%";
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", SID).caller().unwrap()
}
pub(crate) fn directory(count: Option<u64>) -> Value {
    let mut favorite = json!({"id":901,"type":"MYFAVORITE","info":"原文 + %20","pic":"https://img4.kuwo.cn/star/albumcover/f.jpg","ispub":false});
    if let Some(count) = count {
        favorite["musicnum"] = json!(count);
    }
    json!({"errcode":0,"result":"ok","plist":[favorite,
        {"type":"GENERAL","id":101,"title":"我喜欢听","musicnum":7},
        {"type":"MOBI_DEFAULT","id":102,"title":"我喜欢听"}]})
}
pub(crate) fn flow() -> Vec<Vec<u8>> {
    [
        json!({"result":"ok"}),
        directory(Some(4)),
        page(&[11, 22], 2),
        page(&[22, 33], 2),
        page(&[11, 22], 2),
        page(&[22, 33], 2),
        directory(Some(4)),
    ]
    .iter()
    .map(json_response)
    .collect()
}

#[tokio::test]
async fn native_favorites_sdk_uses_system_type_real_cloud_id_and_private_complete_snapshot() {
    let mut f = fixture::setup(flow()).await;
    let metadata = f
        .client
        .native_favorite_playlist(&credential())
        .await
        .unwrap();
    assert_eq!(metadata.id, "901");
    assert_eq!(metadata.name, "我喜欢听");
    assert_eq!(metadata.track_count, Some(4));
    assert_eq!(metadata.extensions["is_favorite"], true);
    assert_eq!(metadata.extensions["upstream_type"], "MYFAVORITE");
    assert_eq!(metadata.extensions["library_section"], "favorite");
    assert_eq!(metadata.extensions["owner_id"], "42");
    assert_eq!(metadata.extensions["backend"], "native_favorite_playlist");
    assert_eq!(metadata.extensions["is_public"], false);
    assert_eq!(metadata.extensions["upstream_pages_fetched"], 6);
    let requests = fixture::requests(&mut f, 7).await;
    for (i, r) in requests.iter().enumerate().skip(1) {
        assert!(r.starts_with("GET "));
        let target = r.split_whitespace().nth(1).unwrap();
        let u = Url::parse(&format!("https://fixture.test{target}")).unwrap();
        let pairs = u.query_pairs().collect::<Vec<_>>();
        let q = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(q.len(), pairs.len());
        assert_eq!(q["uid"], "42");
        assert_eq!(q["sid"], SID);
        assert!(r.to_ascii_lowercase().contains("\r\ncookies: "));
        assert!(!r.to_ascii_lowercase().contains("\r\ncookie: "));
        if (2..=5).contains(&i) {
            assert_eq!(q["pid"], "901");
            assert_eq!(q["sig"], "0");
            assert_eq!(q["op"], "pl3_getlist");
        } else {
            assert_eq!(q["op"], "pl3_getuserlists");
        }
    }
    let mut f = fixture::setup(flow()).await;
    let result = f
        .client
        .native_favorite_tracks(&credential(), &PageRequest::new(2, 1))
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
    assert_eq!(result.pagination.next_offset, Some(3));
    assert_eq!(result.pagination.total, Some(4));
    assert_eq!(
        result.pagination.extensions["source_snapshot_id"],
        metadata.extensions["source_snapshot_id"]
    );
    assert!(
        result
            .items
            .iter()
            .all(|t| t.extensions["backend"] == "native_favorite_playlist" && t.playable.is_none())
    );
    assert!(!serde_json::to_string(&result).unwrap().contains(SID));
    fixture::requests(&mut f, 7).await;
}

#[tokio::test]
async fn native_playlist_content_accepts_plain_text_mime_only_for_json_content() {
    let mut replies = flow();
    replies[2] = response(
        200,
        "text/plain; charset=utf-8",
        "",
        &serde_json::to_vec(&page(&[11, 22], 2)).unwrap(),
    );
    let mut f = fixture::setup(replies).await;
    assert_eq!(
        f.client
            .native_favorite_playlist(&credential())
            .await
            .unwrap()
            .track_count,
        Some(4)
    );
    fixture::requests(&mut f, 7).await;

    let mut replies = flow();
    replies[2] = response(200, "text/plain", "", b"not-json");
    replies.truncate(3);
    let mut f = fixture::setup(replies).await;
    assert!(
        f.client
            .native_favorite_playlist(&credential())
            .await
            .is_err()
    );
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_favorites_directory_rejects_missing_ambiguous_or_invalid_identity() {
    let mut cases = vec![
        json!({"errcode":0,"plist":[]}),
        json!({"errcode":603}),
        json!({"errcode":0,"uid":43,"plist":[]}),
    ];
    for (field, value) in [
        ("id", json!(0)),
        ("id", json!("0901")),
        ("id", json!("9223372036854775808")),
        ("type", json!("GENERAL")),
        ("type", json!("PC_DEFAULT")),
        ("type", json!("UNKNOWN")),
        ("info", json!(SID)),
        ("pic", json!("https://evil.test/f.jpg")),
    ] {
        let mut d = directory(Some(4));
        d["plist"][0][field] = value;
        // Matching the system name does not make an ordinary list the favorite list.
        d["plist"][0]["title"] = json!("我喜欢听");
        cases.push(d);
    }
    let mut d = directory(Some(4));
    let second = d["plist"][0].clone();
    d["plist"].as_array_mut().unwrap().push(second);
    cases.push(d);
    let mut d = directory(Some(4));
    let mut second = d["plist"][0].clone();
    second["id"] = json!(902);
    d["plist"].as_array_mut().unwrap().push(second);
    cases.push(d);
    let mut d = directory(Some(4));
    d["plist"][1]["id"] = json!(901);
    cases.push(d);
    for d in cases {
        let mut f = fixture::setup(vec![
            json_response(&json!({"result":"ok"})),
            json_response(&d),
        ])
        .await;
        let e = f
            .client
            .native_favorite_playlist(&credential())
            .await
            .unwrap_err();
        assert!(!e.to_string().contains(SID));
        fixture::requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn native_favorites_empty_requires_known_zero_and_out_of_range_still_reads_all() {
    for empty in [json!({"errcode":0,"songchange":false}), page(&[], 1)] {
        let mut f = fixture::setup(
            [
                json!({"result":"ok"}),
                directory(Some(0)),
                empty.clone(),
                empty,
                directory(Some(0)),
            ]
            .iter()
            .map(json_response)
            .collect(),
        )
        .await;
        let page = f
            .client
            .native_favorite_tracks(&credential(), &PageRequest::new(10, 50))
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.pagination.total, Some(0));
        fixture::requests(&mut f, 5).await;
    }
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(None)),
        json_response(&json!({"errcode":0,"songchange":false})),
    ])
    .await;
    assert!(
        f.client
            .native_favorite_playlist(&credential())
            .await
            .is_err()
    );
    fixture::requests(&mut f, 3).await;
    let mut f = fixture::setup(flow()).await;
    let p = f
        .client
        .native_favorite_tracks(&credential(), &PageRequest::new(1, 50))
        .await
        .unwrap();
    assert!(p.items.is_empty());
    assert_eq!(p.pagination.total, Some(4));
    fixture::requests(&mut f, 7).await;
}

#[tokio::test]
async fn native_favorites_cannot_change_cloud_identity_type_or_contents_during_read() {
    let mut finals = Vec::new();
    for (key, value) in [
        ("id", json!(902)),
        ("type", json!("GENERAL")),
        ("musicnum", json!(5)),
        ("ispub", json!(true)),
    ] {
        let mut d = directory(Some(4));
        d["plist"][0][key] = value;
        d["plist"][0]["title"] = json!("我喜欢听");
        finals.push(d);
    }
    finals.push(json!({"errcode":0,"plist":[]}));
    for d in finals {
        let mut bodies = flow();
        bodies[6] = json_response(&d);
        let mut f = fixture::setup(bodies).await;
        assert!(
            f.client
                .native_favorite_playlist(&credential())
                .await
                .is_err()
        );
        fixture::requests(&mut f, 7).await;
    }
    for (at, bad, count) in [
        (3, json_response(&page(&[22, 33], 3)), 4),
        (5, json_response(&page(&[22, 99], 2)), 6),
        (3, response(401, "application/json", "", b"private"), 4),
    ] {
        let mut bodies = flow();
        bodies[at] = bad;
        bodies.truncate(count);
        let mut f = fixture::setup(bodies).await;
        assert!(
            f.client
                .native_favorite_playlist(&credential())
                .await
                .is_err()
        );
        fixture::requests(&mut f, count).await;
    }
}

#[tokio::test]
async fn native_favorites_validate_input_and_budget_before_unbounded_reads() {
    let mut f = fixture::setup(vec![]).await;
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert_eq!(
            f.client
                .native_favorite_tracks(&credential(), &PageRequest::new(limit, offset))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut request = PageRequest::new(1, 0);
    request.account = Some("stored".into());
    assert_eq!(
        f.client
            .native_favorite_tracks(&credential(), &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    fixture::requests(&mut f, 0).await;
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(Some(10001))),
    ])
    .await;
    assert!(
        f.client
            .native_favorite_playlist(&credential())
            .await
            .is_err()
    );
    fixture::requests(&mut f, 2).await;
}

#[tokio::test]
async fn native_favorites_remain_excluded_from_ordinary_created_directory_and_sdk() {
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(Some(4))),
    ])
    .await;
    let p = f
        .client
        .native_created_playlists(&credential(), &PageRequest::new(10, 0))
        .await
        .unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].id, "101");
    fixture::requests(&mut f, 2).await;
    let mut f = fixture::setup(vec![
        json_response(&json!({"result":"ok"})),
        json_response(&directory(Some(4))),
    ])
    .await;
    assert_eq!(
        f.client
            .native_created_playlist(&credential(), "901")
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    fixture::requests(&mut f, 2).await;
}
