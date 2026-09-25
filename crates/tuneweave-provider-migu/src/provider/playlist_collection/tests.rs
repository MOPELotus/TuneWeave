use super::*;
use crate::client::account_playlist::tests::metadata;
use crate::credential::MiguCredential;
use crate::provider::{
    catalog::tests::server,
    session::tests::{Store, gated, read, stored},
};
use std::time::Duration;
use tuneweave_core::ErrorCode;

fn profile() -> serde_json::Value {
    json!({"code":"000000","data":{"userId":"111","nickName":"Account"}})
}
fn frames(subscribed: bool) -> Vec<serde_json::Value> {
    let mut result = vec![profile()];
    if subscribed {
        let mut detail = metadata("77", "222", 3);
        detail["title"] = json!("中文 A & B # / 100%");
        result.extend([json!({"code":"000000","data":detail}), profile()]);
    }
    result.extend([json!({"code":"000000"}), profile()]);
    let ids: Vec<u32> = if subscribed {
        std::iter::once(77).chain(1..=20).collect()
    } else {
        (1..=21).collect()
    };
    for chunk in ids.chunks(20) {
        result.extend([json!({"code":"000000","totalCount":21,"collections":chunk.iter().map(|id|json!({"musicListId":id.to_string(),"title":format!("Playlist {id}"),"musicNum":3,"ownerId":"222","ownerName":"Other creator"})).collect::<Vec<_>>()}),profile()]);
    }
    result.extend([json!([{"isOP":if subscribed {"00"}else{"01"},"resourceId":"77","resourceType":"2021","opType":"03","userId":"111"}]),profile()]);
    result
}
fn wire(values: &[serde_json::Value]) -> Vec<String> {
    values.iter().enumerate().map(|(i,v)|{
        let body=v.to_string();format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: p{i}\r\nConnection: close\r\n\r\n{body}",body.len())
    }).collect()
}
fn setup(p: &mut MiguProvider) -> (Arc<Store>, MiguCredential, MiguCredential) {
    let store = Arc::new(Store::default());
    let a = MiguCredential::verified("111".into(), "initial".into()).unwrap();
    let b = MiguCredential::verified("222".into(), "other".into()).unwrap();
    store.put(&stored("A", &a)).unwrap();
    store.put(&stored("B", &b)).unwrap();
    p.credential_store = Some(store.clone());
    (store, a, b)
}
fn is_mutation(request: &str) -> bool {
    request.starts_with("GET /pc/v1.0/user/add_collection.do?")
        || request.starts_with("GET /pc/v1.0/user/del_collection.do?")
}

#[tokio::test]
async fn playlist_collection_uses_exact_get_mutation_parameters_and_complete_double_readback() {
    for caller in [false, true] {
        for subscribed in [false, true] {
            let values = frames(subscribed);
            let count = values.len();
            let (mut p, requests) = server(wire(&values)).await;
            let (store, a, b) = setup(&mut p);
            let alias = if caller {
                p = p.caller_scope(&a.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let result = p
                .set_playlist_subscription("77", subscribed, Some(alias))
                .await
                .unwrap();
            assert_eq!(result.resource_ref.to_string(), "migu:77");
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.extensions["source_user_id"], "111");
            assert!(
                !serde_json::to_string(&result)
                    .unwrap()
                    .contains("pacmtoken")
            );
            let update = p.take_response_credential().unwrap();
            if caller {
                assert_eq!(
                    MiguCredential::parse_caller(&update.unwrap())
                        .unwrap()
                        .token(),
                    format!("p{}", count - 1)
                );
                assert_eq!(read(&store, "A"), a);
            } else {
                assert!(update.is_none());
                assert_eq!(read(&store, "A").token(), format!("p{}", count - 1));
            }
            assert_eq!(read(&store, "B"), b);
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), count);
            let writes: Vec<_> = requests.iter().filter(|r| is_mutation(r)).collect();
            assert_eq!(writes.len(), 1);
            let path = writes[0].split_whitespace().nth(1).unwrap();
            let url = url::Url::parse(&format!("https://app.c.nf.migu.cn{path}")).unwrap();
            let params: std::collections::BTreeMap<_, _> = url
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let expected = if subscribed {
                vec![
                    ("outOPType", "03"),
                    ("outResourceName", "中文 A & B # / 100%"),
                    ("outResourceId", "77"),
                    ("outResourceType", "2021"),
                ]
            } else {
                vec![
                    ("oPType", "03"),
                    ("resourceId", "77"),
                    ("resourceType", "2021"),
                ]
            };
            assert_eq!(
                params,
                expected
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect()
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.starts_with("GET /resource/playlist/v2.0?"))
                    .count(),
                usize::from(subscribed)
            );
            let pages: Vec<_> = requests
                .iter()
                .filter(|r| r.starts_with("GET /user/h5/user/collection/v1.0?"))
                .collect();
            assert_eq!(pages.len(), 2);
            assert!(pages[1].contains("pageNo=2&pageSize=20&OPType=03&resourceType=2021&type=1"));
            assert!(
                requests[count - 2].starts_with("GET /pc/query-ops/2021?opType=03&resourceId=77 ")
            );
            for (i, request) in requests.iter().enumerate() {
                let token = if i == 0 {
                    "initial".to_owned()
                } else {
                    format!("p{}", i - 1)
                };
                assert!(request.contains(&format!("pacmtoken: {token}\r\n")), "{i}");
                assert!(!request.contains("cookie:"));
                assert!(!request.contains("other"));
            }
        }
    }
}

#[tokio::test]
async fn playlist_collection_raw_state_errors_do_not_become_false_or_discard_verified_updates() {
    for caller in [false, true] {
        for subscribed in [false, true] {
            for variant in 0..9 {
                let mut values = frames(subscribed);
                let state_index = values.len() - 2;
                values[state_index] = match variant {
                    0 => json!([]),
                    1 => json!([{"isOP":if subscribed {"01"}else{"00"}}]),
                    2 => json!([{"isOP":"00","resourceId":"88"}]),
                    3 => json!([{"isOP":"02"}]),
                    4 => json!([{"isOP":"00","userId":"222"}]),
                    5 => json!({"code":"200000","info":"missing uid"}),
                    6 => json!({"code":"290001"}),
                    7 => json!({"code":"000000","data":[{"isOP":"00"}]}),
                    _ => json!([{"isOP":"00"},{"isOP":"00"}]),
                };
                if (4..=7).contains(&variant) {
                    values.pop();
                }
                let count = values.len();
                let (mut p, requests) = server(wire(&values)).await;
                let (store, a, b) = setup(&mut p);
                let alias = if caller {
                    p = p.caller_scope(&a.caller().unwrap()).unwrap();
                    "default"
                } else {
                    "A"
                };
                let mut error = p
                    .set_playlist_subscription("77", subscribed, Some(alias))
                    .await
                    .unwrap_err();
                let auth = matches!(variant, 4 | 6);
                assert_eq!(
                    error.code,
                    if auth {
                        ErrorCode::AuthenticationRequired
                    } else {
                        ErrorCode::UpstreamError
                    }
                );
                assert_eq!(error.details["write_outcome"], "unconfirmed");
                assert!(!error.retryable);
                let update = error.take_caller_credential_update();
                let latest = values
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, v)| v["data"]["nickName"].is_string())
                    .map(|(i, _)| format!("p{i}"));
                if caller && !auth {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        latest.unwrap()
                    );
                } else {
                    assert!(update.is_none());
                }
                if caller {
                    assert_eq!(read(&store, "A"), a);
                } else if auth {
                    assert!(
                        !store
                            .load_platform(Platform::Migu)
                            .unwrap()
                            .iter()
                            .any(|v| v.account == "A")
                    );
                }
                assert_eq!(read(&store, "B"), b);
                let requests = requests.await.unwrap();
                assert_eq!(requests.len(), count);
                assert_eq!(requests.iter().filter(|r| is_mutation(r)).count(), 1);
            }
        }
    }
}

#[tokio::test]
async fn playlist_collection_never_stops_at_an_early_match_or_confirms_incomplete_later_pages() {
    for subscribed in [false, true] {
        for variant in 0..3 {
            let mut values = frames(subscribed);
            let last_page = values.len() - 4;
            match variant {
                0 => values[last_page]["collections"] = json!([]),
                1 => values[last_page]["totalCount"] = json!(22),
                _ => values[last_page]
                    .as_object_mut()
                    .unwrap()
                    .remove("collections")
                    .map(|_| ())
                    .unwrap(),
            }
            values.truncate(last_page + 2);
            let count = values.len();
            let (mut p, requests) = server(wire(&values)).await;
            let (_, a, _) = setup(&mut p);
            let p = p.caller_scope(&a.caller().unwrap()).unwrap();
            let mut error = p
                .set_playlist_subscription("77", subscribed, None)
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            assert!(!error.retryable);
            let update = error.take_caller_credential_update().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap().token(),
                format!("p{}", count - 1)
            );
            assert_eq!(
                requests
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| is_mutation(r))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn playlist_collection_write_failures_are_not_retried_and_preflight_errors_do_not_claim_a_write()
 {
    let (mut p, requests) = server(vec![]).await;
    let (_, a, _) = setup(&mut p);
    for id in ["0", "077", "1,2"] {
        assert_eq!(
            p.set_playlist_subscription(id, true, Some("A"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        p.set_playlist_subscription("77", true, Some("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        p.caller_scope(&a.caller().unwrap())
            .unwrap()
            .set_playlist_subscription("77", true, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(requests.await.unwrap().is_empty());
    for subscribed in [false, true] {
        let write_index = if subscribed { 3 } else { 1 };
        for auth in [false, true] {
            let mut values = frames(subscribed);
            values[write_index] = json!({"code":if auth {"290001"}else{"299999"}});
            values.truncate(write_index + 1);
            let (mut p, requests) = server(wire(&values)).await;
            setup(&mut p);
            let e = p
                .set_playlist_subscription("77", subscribed, Some("A"))
                .await
                .unwrap_err();
            assert_eq!(
                e.code,
                if auth {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            assert_eq!(
                requests
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| is_mutation(r))
                    .count(),
                1
            );
        }
    }
    let mut values = frames(true);
    values[1]["data"]["musicListId"] = json!("88");
    values.truncate(3);
    let (mut p, requests) = server(wire(&values)).await;
    setup(&mut p);
    let e = p
        .set_playlist_subscription("77", true, Some("A"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::UpstreamError);
    assert!(e.details.get("write_outcome").is_none());
    assert!(!requests.await.unwrap().iter().any(|r| is_mutation(r)));
}

#[tokio::test]
async fn playlist_collection_late_responses_never_overwrite_or_export_another_login() {
    for subscribed in [false, true] {
        let replies = wire(&frames(subscribed));
        let write_stage = if subscribed { 4 } else { 2 };
        for caller in [false, true] {
            for stage in 1..=replies.len() {
                let (mut p, seen, release, server) = gated(replies[..stage].to_vec()).await;
                let (store, a, b) = setup(&mut p);
                let alias = if caller {
                    p = p.caller_scope(&a.caller().unwrap()).unwrap();
                    "default"
                } else {
                    "A"
                };
                let p = Arc::new(p);
                let worker = p.clone();
                let task = tokio::spawn(async move {
                    worker
                        .set_playlist_subscription("77", subscribed, Some(alias))
                        .await
                });
                tokio::time::timeout(Duration::from_secs(5), seen)
                    .await
                    .unwrap()
                    .unwrap();
                let newer = MiguCredential::verified("111".into(), "new-login".into()).unwrap();
                if caller {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() = newer.clone();
                } else if stage % 2 == 0 {
                    store.remove(Platform::Migu, "A").unwrap();
                } else {
                    store.put(&stored("A", &newer)).unwrap();
                }
                release.send(()).unwrap();
                let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(e.code, ErrorCode::Conflict);
                assert!(e.take_caller_credential_update().is_none());
                assert!(p.take_response_credential().unwrap().is_none());
                assert_eq!(
                    e.details.get("write_outcome").is_some(),
                    stage >= write_stage
                );
                if caller {
                    assert_eq!(read(&store, "A"), a);
                } else if stage % 2 != 0 {
                    assert_eq!(read(&store, "A"), newer);
                } else {
                    assert!(
                        !store
                            .load_platform(Platform::Migu)
                            .unwrap()
                            .iter()
                            .any(|v| v.account == "A")
                    );
                }
                assert_eq!(read(&store, "B"), b);
                server.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn playlist_collection_timeout_at_each_stage_preserves_only_verified_rotations() {
    for subscribed in [false, true] {
        let values = frames(subscribed);
        let replies = wire(&values);
        let write_stage = if subscribed { 4 } else { 2 };
        for stage in 1..=replies.len() {
            let (mut p, seen, release, server) = gated(replies[..stage].to_vec()).await;
            let (_, a, _) = setup(&mut p);
            p.client = p
                .client
                .with_session_test_timeout(Duration::from_millis(300));
            let p = Arc::new(p.caller_scope(&a.caller().unwrap()).unwrap());
            let worker = p.clone();
            let task = tokio::spawn(async move {
                worker
                    .set_playlist_subscription("77", subscribed, None)
                    .await
            });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::UpstreamTimeout);
            assert_eq!(
                e.details.get("write_outcome").is_some(),
                stage >= write_stage
            );
            if stage >= write_stage {
                assert!(!e.retryable);
            }
            let expected = values[..stage - 1]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, v)| v["data"]["nickName"].is_string())
                .map(|(i, _)| format!("p{i}"));
            assert_eq!(
                e.take_caller_credential_update()
                    .map(|v| MiguCredential::parse_caller(&v).unwrap().token().to_owned()),
                expected
            );
            assert_eq!(
                p.take_response_credential()
                    .unwrap()
                    .map(|v| MiguCredential::parse_caller(&v).unwrap().token().to_owned()),
                expected
            );
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
            drop(release);
        }
    }
}
