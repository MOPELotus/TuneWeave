use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::credential::NativeCredential,
    native::tests as fixture,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn entry(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "AARTIST": "Alias",
        "img": "https://star.kuwo.cn/star/starheads/180/test.jpg",
        "musiccnt": "2258",
        "albumcnt": "207",
        "mvcnt": "",
        "followers": "123863",
        "digest": "4",
        "szb": "0"
    })
}

fn body(entries: Vec<Value>) -> Value {
    json!({"result":"ok", "uid":"42", "total":entries.len(), "data":entries})
}

fn input() -> KuwoNativeSessionInput {
    let credential = fixture::credential_fixture("42", "private-library-session")
        .caller()
        .unwrap();
    NativeCredential::parse(&credential)
        .unwrap()
        .input()
        .unwrap()
}

fn parse_value(value: &Value) -> Result<Vec<Artist>> {
    parse(&serde_json::to_vec(value).unwrap(), &input())
}

#[test]
fn following_artists_map_only_typed_artist_identity_and_safe_profile_fields() {
    let artists = parse_value(&body(vec![entry("810", "谭咏麟"), entry("336", "周杰伦")])).unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0].id, "810");
    assert_eq!(artists[0].name, "谭咏麟");
    assert_eq!(artists[0].aliases, ["Alias"]);
    assert_eq!(artists[0].track_count, Some(2258));
    assert_eq!(artists[0].album_count, Some(207));
    assert_eq!(artists[0].mv_count, None);
    assert_eq!(artists[0].extensions["followers"], 123863);
    assert_eq!(
        artists[0].avatar_url.as_deref(),
        Some("https://star.kuwo.cn/star/starheads/180/test.jpg")
    );
    assert_eq!(artists[0].platform, Platform::Kuwo);
    assert!(artists.iter().all(|artist| artist.description.is_empty()));
    assert!(artists.iter().all(|artist| artist.cover_url.is_none()));

    let empty = parse_value(&json!({"result":"ok","data":[]})).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn following_artists_reject_ambiguous_identity_partial_and_unsupported_rows() {
    for bad in [
        json!({}),
        json!({"data": []}),
        json!({"result":"error","data": []}),
        json!({"result":"ok"}),
        json!({"result":"ok","errcode":6,"data": []}),
        json!({"result":"ok","uid":"43","data": []}),
        body(vec![json!({"id":"810","name":"缺少 digest"})]),
        body(vec![{
            let mut row = entry("810", "谭咏麟");
            row["digest"] = json!("8");
            row
        }]),
        body(vec![{
            let mut row = entry("810", "星火社");
            row["szb"] = json!("1");
            row
        }]),
        body(vec![entry("0", "坏 ID")]),
        body(vec![entry("0810", "非规范 ID")]),
        body(vec![entry("810", "")]),
        body(vec![{
            let mut row = entry("810", "周杰伦");
            row["musiccnt"] = json!("1.5");
            row
        }]),
        body(vec![{
            let mut row = entry("810", "周杰伦");
            row["AARTIST"] = json!(input().session_id());
            row
        }]),
        body(vec![{
            let mut row = entry("810", "周杰伦");
            row["img"] = json!(
                "https://star.kuwo.cn/star/starheads/180/test.jpg?sid=private-library-session"
            );
            row
        }]),
        body(vec![entry("810", "周杰伦"), entry("810", "周杰伦")]),
    ] {
        assert!(parse_value(&bad).is_err(), "{bad}");
    }

    let full = body(
        (0..REQUEST_SIZE)
            .map(|id| entry(&id.to_string(), "Artist"))
            .collect(),
    );
    assert!(parse_value(&full).is_err());
}

#[tokio::test]
async fn following_artists_fetches_fixed_self_endpoint_and_slices_only_after_complete_read() {
    let mut f = fixture::setup(vec![json_response(&body(vec![
        entry("810", "谭咏麟"),
        entry("336", "周杰伦"),
        entry("312", "动力火车"),
    ]))])
    .await;
    let page = f
        .client
        .fetch_following_artists(&input(), &PageRequest::new(2, 1), || Ok(()))
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["336", "312"]
    );
    assert_eq!(page.pagination.total, Some(3));
    assert_eq!(page.pagination.next_offset, None);
    assert!(!page.pagination.has_more);
    assert_eq!(page.pagination.extensions["library_owner_id"], "42");

    let requests = fixture::requests(&mut f, 1).await;
    let request = &requests[0];
    let target = request.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("https://fixture.test{target}")).unwrap();
    assert_eq!(url.path(), PATH);
    let pairs = url.query_pairs().collect::<Vec<_>>();
    let query = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
    assert_eq!(pairs.len(), query.len());
    assert_eq!(query["type"], "get_like_list");
    assert_eq!(query["uid"], "42");
    assert_eq!(query["digest"], "4");
    assert_eq!(query["start"], "0");
    assert_eq!(query["count"], "1000");
    assert_eq!(query["loginSid"], input().session_id());
    assert_eq!(query["newver"], "3");
    assert!(request.contains("loginUid=42,loginSid=private-library-session"));
}

#[tokio::test]
async fn following_artists_rejects_bad_windows_before_network_and_keeps_empty_distinct() {
    let mut f = fixture::setup(vec![]).await;
    for request in [
        PageRequest::new(0, 0),
        PageRequest::new(101, 0),
        PageRequest::new(1, u32::MAX),
    ] {
        assert!(
            f.client
                .fetch_following_artists(&input(), &request, || Ok(()))
                .await
                .is_err()
        );
    }
    assert!(fixture::requests(&mut f, 0).await.is_empty());

    let mut f = fixture::setup(vec![json_response(&json!({"result":"ok","data":[]}))]).await;
    let page = f
        .client
        .fetch_following_artists(&input(), &PageRequest::new(10, 100), || Ok(()))
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.pagination.total, Some(0));
    assert!(!page.pagination.has_more);
    fixture::requests(&mut f, 1).await;
}

#[tokio::test]
async fn following_artist_write_uses_the_official_single_artist_quanzi_request() {
    for subscribed in [true, false] {
        let mut f = fixture::setup(vec![json_response(&json!({"result":"ok"}))]).await;
        let mut dispatched = false;
        f.client
            .set_following_artist(&input(), "810", subscribed, &mut dispatched, || Ok(()))
            .await
            .unwrap();
        assert!(dispatched);

        let requests = fixture::requests(&mut f, 1).await;
        let request = &requests[0];
        assert!(request.starts_with("GET "));
        assert!(request.contains("loginUid=42,loginSid=private-library-session"));
        let target = request.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("https://fixture.test{target}")).unwrap();
        assert_eq!(url.path(), PATH);
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let query = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(pairs.len(), 6);
        assert_eq!(
            query["type"],
            if subscribed {
                "click_like"
            } else {
                "cancel_like"
            }
        );
        assert_eq!(query["uid"], "42");
        assert_eq!(query["digest"], "4");
        assert_eq!(query["sid"], "810");
        assert_eq!(query["loginSid"], input().session_id());
        assert_eq!(query["newver"], "3");
    }
}

#[tokio::test]
async fn following_artist_write_rejects_bad_ids_and_stale_selection_before_dispatch() {
    for id in ["", "0", "0810", "810x", "18446744073709551616"] {
        let mut f = fixture::setup(vec![]).await;
        let mut dispatched = false;
        assert!(
            f.client
                .set_following_artist(&input(), id, true, &mut dispatched, || Ok(()))
                .await
                .is_err()
        );
        assert!(!dispatched);
        assert!(fixture::requests(&mut f, 0).await.is_empty());
    }

    let mut f = fixture::setup(vec![]).await;
    let mut dispatched = false;
    assert_eq!(
        f.client
            .set_following_artist(&input(), "810", true, &mut dispatched, || {
                Err(TuneWeaveError::new(
                    ErrorCode::Conflict,
                    "selection changed",
                ))
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(!dispatched);
    assert!(fixture::requests(&mut f, 0).await.is_empty());
}

#[tokio::test]
async fn following_artist_write_rejects_an_empty_success_body() {
    let mut f = fixture::setup(vec![response(200, "application/json", "", b"")]).await;
    let mut dispatched = false;
    assert!(
        f.client
            .set_following_artist(&input(), "810", true, &mut dispatched, || Ok(()))
            .await
            .is_err()
    );
    assert!(dispatched);
    fixture::requests(&mut f, 1).await;
}
