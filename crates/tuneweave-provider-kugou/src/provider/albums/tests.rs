use super::super::session::tests::{Store, credential, raw, server};
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn detail() -> Value {
    json!({"status":1,"error_code":0,"errcode":0,"data":[{"album_id":"42","album_name":"Album","authors":[{"author_id":"7","author_name":"A"}]}]})
}
fn song(n: u64) -> Value {
    json!({"base":{"album_id":42,"album_audio_id":100+n,"audio_id":200+n,"audio_name":format!("Song {n}")},"extend":{"disc":if n<13 {1}else{2},"sort":n+1},"authors":[{"author_id":7,"author_name":"A"}]})
}
fn page(p: u32, total: u64) -> Value {
    let start = u64::from(p - 1) * 20;
    json!({"status":1,"error_code":0,"total":total,"extra":{"disc_cnt":2},"data":{"total":total,"songs":(start..total.min(start+20)).map(song).collect::<Vec<_>>()}})
}
fn request(offset: u32, limit: u32) -> PageRequest {
    PageRequest {
        offset,
        limit,
        account: None,
    }
}

#[tokio::test]
async fn albums_fetch_complete_catalogues_before_slicing_and_never_use_stored_accounts() {
    let mut f = server(vec![
        raw(detail()).into(),
        raw(page(1, 25)).into(),
        raw(page(2, 25)).into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    let old = credential("999", "account-secret");
    store.put(&old.stored("default").unwrap()).unwrap();
    f.provider.credential_store = Some(store.clone());
    let p = f
        .provider
        .album_tracks("42", &request(18, 5))
        .await
        .unwrap();
    assert_eq!(
        p.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["118", "119", "120", "121", "122"]
    );
    assert_eq!(p.pagination.total, Some(25));
    assert_eq!(p.pagination.next_offset, Some(23));
    assert_eq!(p.pagination.extensions["complete_snapshot"], true);
    assert_eq!(p.pagination.extensions["disc_count"], 2);
    assert_eq!(p.items[0].extensions["album_position"], 18);
    assert_eq!(p.items[0].extensions["disc_number"], 2);
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 3);
    let mut identity = None;
    for (i, r) in requests.iter().enumerate() {
        let (head, body) = r.split_once("\r\n\r\n").unwrap();
        let target = head
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
        let mut q: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        let signature = q.remove("signature").unwrap();
        let borrowed = q.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        assert_eq!(
            signature,
            crate::signing::android_signature(&borrowed, body.as_bytes())
        );
        assert_eq!(q.len(), 8);
        assert_eq!(q["appid"], "1005");
        assert_eq!(q["clientver"], "20489");
        assert_eq!(q["userid"], "0");
        assert_eq!(q["token"], "");
        assert_eq!(q["dfid"], "-");
        assert_eq!(q["uuid"].len(), 36);
        let current = (q["mid"].clone(), q["uuid"].clone());
        if let Some(old) = &identity {
            assert_eq!(old, &current);
        } else {
            identity = Some(current);
        }
        assert!(!head.to_lowercase().contains("cookie:"));
        assert!(!head.to_lowercase().contains("authorization:"));
        assert!(!r.contains("account-secret"));
        assert!(head.to_lowercase().contains("kg-tid: 255"));
        let b: Value = serde_json::from_str(body).unwrap();
        if i == 0 {
            assert_eq!(url.path(), "/kmr/v2/albums");
            assert_eq!(b["data"][0]["album_id"], 42);
            assert_eq!(b["is_buy"], 0);
            assert!(b["fields"].as_str().unwrap().contains("publish_company"));
        } else {
            assert_eq!(url.path(), "/v1/album_audio/lite");
            assert_eq!(b, json!({"album_id":42,"is_buy":"","page":i,"pagesize":20}));
        }
    }
    assert_eq!(
        store.values.lock().unwrap().get("default").unwrap(),
        &old.stored("default").unwrap()
    );
}

#[tokio::test]
async fn albums_keep_duplicate_tracks_in_distinct_slots_and_handle_empty_and_out_of_range_pages() {
    let mut p = page(1, 2);
    p["data"]["songs"][1]["base"]["album_audio_id"] = json!(100);
    let f = server(vec![raw(detail()).into(), raw(p).into()]).await;
    let p = f
        .provider
        .album_tracks("42", &request(0, 10))
        .await
        .unwrap();
    assert_eq!(p.items.len(), 2);
    assert_eq!(p.items[0].id, p.items[1].id);
    assert_ne!(
        p.items[0].extensions["track_number"],
        p.items[1].extensions["track_number"]
    );
    assert!(!p.pagination.has_more);
    f.requests.await.unwrap();
    for (total, offset) in [(0, 0), (2, 20)] {
        let f = server(vec![raw(detail()).into(), raw(page(1, total)).into()]).await;
        let p = f
            .provider
            .album_tracks("42", &request(offset, 5))
            .await
            .unwrap();
        assert!(p.items.is_empty());
        assert_eq!(p.pagination.total, Some(total));
        assert_eq!(p.pagination.next_offset, None);
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
    let f = server(vec![raw(detail()).into()]).await;
    assert_eq!(f.provider.album("42", None).await.unwrap().id, "42");
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn albums_discard_partial_catalogues_on_total_disc_identity_and_page_conflicts() {
    for case in 0..7 {
        let first = page(1, 40);
        let mut second = page(2, 40);
        match case {
            0 => {
                second["total"] = json!(41);
                second["data"]["total"] = json!(41);
            }
            1 => second["extra"]["disc_cnt"] = json!(3),
            2 => second = first.clone(),
            3 => second["data"]["songs"][0]["base"]["album_id"] = json!(99),
            4 => second["data"]["songs"] = json!([]),
            5 => second["data"]["songs"][0]["extend"] = json!({"disc":1,"sort":1}),
            6 => second = json!({"status":0,"error_code":20006,"data":"private-error"}),
            _ => unreachable!(),
        }
        let f = server(vec![
            raw(detail()).into(),
            raw(first).into(),
            raw(second).into(),
        ])
        .await;
        let e = f
            .provider
            .album_tracks("42", &request(0, 5))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(!format!("{e:?}").contains("private-error"));
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let f = server(vec![raw(detail()).into(), raw(page(1, 1281)).into()]).await;
    assert_eq!(
        f.provider
            .album_tracks("42", &request(0, 5))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(f.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn albums_reject_invalid_sources_and_pagination_before_network() {
    let f = server(vec![]).await;
    for id in [
        "",
        "0",
        "042",
        "+42",
        " 42",
        "-1",
        "18446744073709551616",
        "https://untrusted.invalid",
    ] {
        assert_eq!(
            f.provider.album(id, None).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .album_tracks(id, &request(0, 10))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .album("42", Some("other"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for (offset, limit) in [(0, 0), (0, 101), (u32::MAX, 1)] {
        assert_eq!(
            f.provider
                .album_tracks("42", &request(offset, limit))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut r = request(0, 5);
    r.account = Some("other".into());
    assert_eq!(
        f.provider.album_tracks("42", &r).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    let scoped = f
        .provider
        .caller_scope(&credential("111", "caller-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        scoped.album("42", None).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        scoped
            .album_tracks("42", &request(0, 5))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn albums_reject_transport_failures_without_redirects_retries_or_partial_results() {
    let good = raw(detail());
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        good.replace("Content-Type:","SSA-CODE: private-challenge\r\nContent-Type:"),
        good.replace("application/json","text/html"),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n".to_owned()+&"x".repeat(1048577),
    ] {
        let f=server(vec![response.into()]).await;let e=f.provider.album("42",None).await.unwrap_err();assert!(!format!("{e:?}").contains("private-"));assert_eq!(f.requests.await.unwrap().len(),1);
    }
    let f = server(vec![
        raw(detail()).into(),
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_owned()
            .into(),
    ])
    .await;
    assert!(f.provider.album_tracks("42", &request(0, 5)).await.is_err());
    assert_eq!(f.requests.await.unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "uses live official anonymous album metadata and track catalogue"]
async fn live_public_album_catalogue_preserves_complete_multi_disc_tracks_and_detail_identity() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    let album = provider.album("2996504", None).await.unwrap();
    assert_eq!(album.id, "2996504");
    assert!(!album.artists.is_empty());
    assert!(album.company.is_some());
    let tracks = provider
        .album_tracks("2996504", &request(0, 100))
        .await
        .unwrap();
    assert_eq!(tracks.pagination.total, Some(tracks.items.len() as u64));
    assert!(!tracks.pagination.has_more);
    assert!(tracks.items.len() > 20);
    assert!(
        tracks
            .items
            .iter()
            .any(|t| t.extensions["disc_number"] == 2)
    );
    for (i, t) in tracks.items.iter().enumerate() {
        assert_eq!(
            t.album
                .as_ref()
                .unwrap()
                .resource_ref
                .as_ref()
                .unwrap()
                .id(),
            album.id
        );
        assert_eq!(t.extensions["album_position"], i);
        assert!(t.duration_ms.is_some());
        assert_eq!(t.playable, None);
    }
    let first = &tracks.items[0];
    let detail = provider.track(&first.id, None).await.unwrap();
    assert_eq!(detail.resource_ref, first.resource_ref);
    assert_eq!(
        detail.album.as_ref().unwrap().resource_ref,
        first.album.as_ref().unwrap().resource_ref
    );
    assert_eq!(detail.name, first.name);
}
