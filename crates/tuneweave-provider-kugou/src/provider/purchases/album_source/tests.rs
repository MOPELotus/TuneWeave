use super::*;
use crate::KugouLoginClient;
use crate::provider::library::tests::store_account;
use crate::provider::purchases::tests::start;
use crate::provider::session::tests::{Frame, paused, raw, read, server};
use serde_json::Value;

fn rows(count: u64) -> Vec<Value> {
    (1..=count)
        .map(|n| {
            let id = if n % 2 == 1 { 42 } else { 84 };
            json!({"id":n,"album_id":id,"album_name":format!("Album {id}"),
                "singer_name":"Synthetic singer","buy_total":if n==1 {99}else{1},
                "is_publish":1,"mp_count":0})
        })
        .collect()
}
fn purchase_pages(rows: &[Value]) -> Vec<Value> {
    let count = rows.len().div_ceil(15).max(1);
    (0..count)
        .map(|page| {
            let start = (page * 15).min(rows.len());
            let end = (start + 15).min(rows.len());
            json!({"status":1,"error_code":0,"data":{"userid":111,
                "page":page+1,"pagesize":15,"total":rows.len(),"goods":rows[start..end]}})
        })
        .collect()
}
fn detail(id: u64) -> Value {
    json!({"status":1,"error_code":0,"data":[{"album_id":id,
        "album_name":format!("Album {id}"),"author_name":"Synthetic singer"}]})
}
fn song(id: u64, index: u64) -> Value {
    json!({"base":{"album_id":id,"album_audio_id":id*1000+if index==20 {0}else{index},
        "audio_name":format!("Song {id}/{index}")},
        "album_info":{"album_id":id,"album_name":format!("Album {id}")},
        "extend":{"disc":if index<20 {1}else{2},"sort":index+1}})
}
fn catalogue(id: u64, total: u64) -> Vec<Value> {
    let mut bodies = vec![detail(id)];
    for page in 0..total.div_ceil(20).max(1) {
        bodies.push(json!({"status":1,"error_code":0,"total":total,
            "extra":{"disc_cnt":if total>20 {2}else{1}},
            "data":{"total":total,"songs":(page*20..total.min(page*20+20))
                .map(|n|song(id,n)).collect::<Vec<_>>()}}));
    }
    bodies
}
fn source_bodies(rows: &[Value]) -> Vec<Value> {
    let mut bodies = purchase_pages(rows);
    let mut seen = BTreeSet::new();
    for row in rows {
        let id = row["album_id"].as_u64().unwrap();
        if seen.insert(id) {
            bodies.extend(catalogue(id, if id == 42 { 21 } else { 2 }));
        }
    }
    bodies.extend(purchase_pages(rows));
    bodies
}
fn frames(bodies: Vec<Value>) -> Vec<Frame> {
    let mut frames = start();
    frames.extend(bodies.into_iter().map(|body| raw(body).into()));
    frames
}
fn request(account: Option<&str>, offset: u32, limit: u32) -> PageRequest {
    PageRequest {
        offset,
        limit,
        account: account.map(str::to_owned),
    }
}

#[tokio::test]
async fn purchased_albums_expand_complete_catalogues_once_per_id_preserving_every_occurrence() {
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for caller in [false, true] {
            let bodies = source_bodies(&rows(16));
            let bytes = bodies
                .iter()
                .map(|body| body.to_string().len())
                .sum::<usize>();
            let mut all_frames = frames(bodies.clone());
            all_frames.extend(frames(bodies));
            let mut f = server(all_frames).await;
            let store = store_account(&mut f.provider);
            let mut session = read(&store, "A").native().session.clone();
            session.client = client;
            store
                .put(
                    &KugouCredential::verified(session)
                        .unwrap()
                        .stored("A")
                        .unwrap(),
                )
                .unwrap();
            let original = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider
                    .caller_scope(&original.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let account = if caller { None } else { Some("A") };
            let metadata = provider
                .playlist_source("111", SOURCE, account)
                .await
                .unwrap();
            let page = provider
                .playlist_source_items("111", SOURCE, &request(account, 20, 5))
                .await
                .unwrap();
            assert_eq!(metadata.track_count, Some(184));
            assert_eq!(metadata.extensions["album_count"], 16);
            assert_eq!(metadata.extensions["unique_album_count"], 2);
            assert_eq!(metadata.extensions["catalogue_requests"], 5);
            assert_eq!(metadata.extensions["purchase_pages_fetched"], 4);
            assert_eq!(metadata.extensions["response_bytes"], bytes);
            assert_eq!(
                metadata.extensions["source_snapshot_id"],
                page.pagination.extensions["source_snapshot_id"]
            );
            assert_eq!(page.pagination.total, Some(184));
            assert_eq!(page.pagination.next_offset, Some(25));
            let tracks = page
                .items
                .iter()
                .map(|item| match item {
                    PlaylistPlayableItem::Track(track) => track,
                    _ => panic!("album source must yield tracks"),
                })
                .collect::<Vec<_>>();
            assert_eq!(
                tracks
                    .iter()
                    .map(|track| track.id.as_str())
                    .collect::<Vec<_>>(),
                ["42000", "84000", "84001", "42000", "42001"]
            );
            assert_eq!(tracks[0].extensions["purchase_album_track_position"], 20);
            assert_eq!(tracks[3].extensions["purchase_album_position"], 2);
            for (index, track) in tracks.iter().enumerate() {
                assert_eq!(track.extensions["source_position"], index + 20);
                assert_eq!(track.playable, None);
                assert!(track.available_qualities.is_empty());
            }
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), original);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), 22);
            for base in [0, 11] {
                for n in 4..=8 {
                    let (head, body) = requests[base + n].split_once("\r\n\r\n").unwrap();
                    let target = head
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
                    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
                    assert_eq!(query["userid"], "0");
                    assert_eq!(query["token"], "");
                    assert!(!head.to_ascii_lowercase().contains("cookie:"));
                    assert!(!head.to_ascii_lowercase().contains("authorization:"));
                    assert!(!requests[base + n].contains("next"));
                    let body: Value = serde_json::from_str(body).unwrap();
                    assert!(body.get("token").is_none());
                }
                for n in [2, 3, 9, 10] {
                    let body: Value =
                        serde_json::from_str(requests[base + n].split_once("\r\n\r\n").unwrap().1)
                            .unwrap();
                    assert_eq!(body["token"], "next");
                    if client == KugouLoginClient::Standard {
                        assert_eq!(body["use_custom_sort"], 1);
                    } else {
                        assert!(body.get("use_custom_sort").is_none());
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn purchased_albums_reject_unresolved_records_and_invalid_sources_before_catalogue_fallback()
{
    let mut f = server(vec![]).await;
    let store = store_account(&mut f.provider);
    for (uid, code) in [
        ("222", ErrorCode::PermissionDenied),
        ("0111", ErrorCode::InvalidRequest),
    ] {
        assert_eq!(
            f.provider
                .playlist_source(uid, SOURCE, Some("A"))
                .await
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(
            f.provider
                .playlist_source_items(uid, SOURCE, &request(Some("A"), 0, 1))
                .await
                .unwrap_err()
                .code,
            code
        );
    }
    assert_eq!(
        f.provider
            .playlist_source_items("111", SOURCE, &request(Some("A"), 0, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let web =
        KugouCredential::verified_web(crate::web::WebSession::test_session("111", "web-secret"))
            .unwrap();
    store.put(&web.stored("A").unwrap()).unwrap();
    assert_eq!(
        f.provider
            .playlist_source("111", SOURCE, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());

    for duplicate in [false, true] {
        let mut rows = rows(16);
        if duplicate {
            rows[15] = rows[0].clone();
        } else {
            rows[15].as_object_mut().unwrap().remove("album_id");
        }
        let mut f = server(frames(purchase_pages(&rows))).await;
        store_account(&mut f.provider);
        let error = f
            .provider
            .playlist_source_items("111", SOURCE, &request(Some("A"), 0, 1))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            if duplicate {
                ErrorCode::Conflict
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
    let rows = rows(1);
    let mut bodies = purchase_pages(&rows);
    let mut invalid = catalogue(42, 21);
    invalid[2]["data"]["songs"][0]["base"]["album_id"] = json!(84);
    bodies.extend(invalid);
    let mut f = server(frames(bodies)).await;
    store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .playlist_source("111", SOURCE, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(f.requests.await.unwrap().len(), 6);
}

#[tokio::test]
async fn purchased_albums_compare_complete_purchase_order_mapping_quantity_and_catalogue_snapshots()
{
    let rows = rows(16);
    for case in ["order", "quantity", "mapping"] {
        let mut confirmation = rows.clone();
        match case {
            "order" => confirmation.swap(14, 15),
            "quantity" => confirmation[0]["buy_total"] = json!(100),
            _ => {
                confirmation[15].as_object_mut().unwrap().remove("album_id");
            }
        }
        let mut bodies = source_bodies(&rows);
        bodies.truncate(bodies.len() - 2);
        bodies.extend(purchase_pages(&confirmation));
        let mut f = server(frames(bodies)).await;
        let store = store_account(&mut f.provider);
        let provider = f
            .provider
            .caller_scope(&read(&store, "A").caller().unwrap())
            .unwrap();
        let mut error = provider
            .playlist_source("111", SOURCE, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict, "{case}");
        assert!(error.take_caller_credential_update().is_none());
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(f.requests.await.unwrap().len(), 11);
    }
    let bodies = source_bodies(&rows);
    let mut changed = bodies.clone();
    changed[3]["data"]["songs"][0]["base"]["audio_name"] = json!("Changed public title");
    let mut all_frames = frames(bodies);
    all_frames.extend(frames(changed));
    let mut f = server(all_frames).await;
    store_account(&mut f.provider);
    let metadata = f
        .provider
        .playlist_source("111", SOURCE, Some("A"))
        .await
        .unwrap();
    let page = f
        .provider
        .playlist_source_items("111", SOURCE, &request(Some("A"), 0, 1))
        .await
        .unwrap();
    assert_ne!(
        metadata.extensions["source_snapshot_id"],
        page.pagination.extensions["source_snapshot_id"]
    );
    assert_eq!(f.requests.await.unwrap().len(), 22);
}

#[tokio::test]
async fn purchased_albums_same_uid_relogin_wins_over_late_catalogue_or_confirmation_results() {
    for index in [2, 4, 8] {
        for failure in [false, true] {
            let bodies = source_bodies(&rows(16));
            let response = if failure {
                raw(json!({"status":0,"error_code":20017}))
            } else {
                raw(bodies[index].clone())
            };
            let (last, resume) = paused(response);
            let mut all_frames = frames(bodies[..index].to_vec());
            all_frames.push(last);
            let mut f = server(all_frames).await;
            let store = store_account(&mut f.provider);
            let provider = f.provider.clone();
            let task =
                tokio::spawn(
                    async move { provider.playlist_source("111", SOURCE, Some("A")).await },
                );
            for _ in 0..index + 3 {
                f.seen.recv().await.unwrap();
            }
            // Same UID, token and device, but a fresh login generation under alias A.
            let replacement =
                KugouCredential::verified(read(&store, "A").native().session.clone()).unwrap();
            store.put(&replacement.stored("A").unwrap()).unwrap();
            resume.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert_eq!(read(&store, "A"), replacement);
            assert!(error.take_caller_credential_update().is_none());
            assert!(f.provider.take_response_credential().unwrap().is_none());
            assert_eq!(f.requests.await.unwrap().len(), index + 3);
        }
    }
    let bodies = source_bodies(&rows(16));
    let (last, resume) = paused(raw(bodies[2].clone()));
    let mut all_frames = frames(bodies[..2].to_vec());
    all_frames.push(last);
    let mut f = server(all_frames).await;
    store_account(&mut f.provider);
    let provider = f.provider.clone();
    let task =
        tokio::spawn(async move { provider.playlist_source("111", SOURCE, Some("A")).await });
    for _ in 0..5 {
        f.seen.recv().await.unwrap();
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    resume.send(()).unwrap();
    assert_eq!(f.requests.await.unwrap().len(), 5);
}

#[tokio::test]
async fn purchased_albums_verify_empty_catalogues_and_enforce_aggregate_budgets() {
    for empty_source in [false, true] {
        let rows = if empty_source { vec![] } else { rows(1) };
        let mut bodies = purchase_pages(&rows);
        if !empty_source {
            bodies.extend(catalogue(42, 0));
        }
        bodies.extend(purchase_pages(&rows));
        let count = bodies.len() + 2;
        let mut f = server(frames(bodies)).await;
        store_account(&mut f.provider);
        let page = f
            .provider
            .playlist_source_items("111", SOURCE, &request(Some("A"), 99, 10))
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.pagination.total, Some(0));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["album_count"], rows.len());
        assert_eq!(f.requests.await.unwrap().len(), count);
    }
    let purchases = rows(1);
    let bodies = source_bodies(&purchases);
    // Count physical JSON bytes, including an otherwise ignored envelope field.
    let mut padded = bodies.clone();
    padded[1]["padding"] = json!("x".repeat(100));
    let first_bytes = padded[0].to_string().len();
    let metadata_bytes = padded[1].to_string().len();
    let repeated = source_bodies(&rows(16));
    for (limits, responses) in [
        (
            Limits {
                albums: 0,
                ..LIMITS
            },
            bodies[..1].to_vec(),
        ),
        (
            Limits {
                tracks: 20,
                ..LIMITS
            },
            bodies[..4].to_vec(),
        ),
        (
            Limits {
                bytes: first_bytes + metadata_bytes - 1,
                ..LIMITS
            },
            padded[..2].to_vec(),
        ),
        (
            Limits {
                tracks: 30,
                ..LIMITS
            },
            repeated[..7].to_vec(),
        ),
    ] {
        let count = responses.len() + 2;
        let mut f = server(frames(responses)).await;
        let store = store_account(&mut f.provider);
        let provider = f
            .provider
            .caller_scope(&read(&store, "A").caller().unwrap())
            .unwrap();
        let mut error = provider
            .purchased_album_snapshot("111", None, limits)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(error.take_caller_credential_update().is_some());
        assert_eq!(f.requests.await.unwrap().len(), count);
    }
    let (last, resume) = paused(raw(bodies[1].clone()));
    let mut all_frames = frames(bodies[..1].to_vec());
    all_frames.push(last);
    let mut f = server(all_frames).await;
    let store = store_account(&mut f.provider);
    let original = read(&store, "A");
    let provider = f
        .provider
        .caller_scope(&original.caller().unwrap())
        .unwrap();
    let task = tokio::spawn(async move {
        provider
            .purchased_album_snapshot(
                "111",
                None,
                Limits {
                    time: Duration::from_secs(1),
                    ..LIMITS
                },
            )
            .await
    });
    for _ in 0..4 {
        f.seen.recv().await.unwrap();
    }
    let mut error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamTimeout);
    let update = error.take_caller_credential_update().unwrap();
    assert_eq!(
        KugouCredential::parse_caller(&update)
            .unwrap()
            .native()
            .session
            .token,
        "next"
    );
    assert_eq!(read(&store, "A"), original);
    resume.send(()).unwrap();
    assert_eq!(f.requests.await.unwrap().len(), 4);
}
