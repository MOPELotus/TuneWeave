use super::*;
use crate::client::account_playlist::tests::{home, metadata, page};
use crate::provider::session::tests::{Store, gated, read, stored};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::ErrorCode;

#[tokio::test]
async fn favorite_write_absent_track_and_consistently_replaced_favorite_id_are_not_confirmation() {
    for variant in 0..3 {
        let subscribed = variant == 2;
        let mut values = frames(subscribed, if subscribed { &[1] } else { &[] });
        let count = if subscribed {
            // A consistent new collection is not the identity checked before writing.
            values[8] = data(home("88"));
            values[10]["data"]["musicListId"] = json!("88");
            values[12]["data"]["playlistId"] = json!("88");
            values[14]["data"]["musicListId"] = json!("88");
            values[16] = data(home("88"));
            18
        } else {
            // Absence alone cannot prove removal when the state is true or unknown.
            values[18]["isInfavors"] = if variant == 0 {
                json!([{"contentId":"1","isInfavor":"1"}])
            } else {
                json!([])
            };
            20
        };
        values.truncate(count);
        let (mut p, requests) = server(wire(&values)).await;
        let (store, a, b) = setup(&mut p);
        let p = p.caller_scope(&a.caller().unwrap()).unwrap();
        let mut error = p
            .set_track_subscription("1", subscribed, None)
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
        assert_eq!(read(&store, "A"), a);
        assert_eq!(read(&store, "B"), b);
        assert_eq!(
            requests
                .await
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            1
        );
    }
}

fn profile() -> serde_json::Value {
    json!({"code":"000000","data":{"userId":"111","nickName":"Account"}})
}
fn data(value: serde_json::Value) -> serde_json::Value {
    json!({"code":"000000","data":value})
}
fn frames(subscribed: bool, ids: &[u32]) -> Vec<serde_json::Value> {
    let mut result = vec![
        profile(),
        data(home("77")),
        profile(),
        data(metadata("77", "111", 0)),
        profile(),
        json!({"code":"000000"}),
        profile(),
    ];
    result.extend([
        profile(),
        data(home("77")),
        profile(),
        data(metadata("77", "111", ids.len())),
        profile(),
    ]);
    if ids.is_empty() {
        result.extend([data(page(&[], 0)), profile()]);
    } else {
        for chunk in ids.chunks(50) {
            result.extend([data(page(chunk, ids.len())), profile()]);
        }
    }
    result.extend([data(metadata("77","111",ids.len())), profile(), data(home("77")), profile(),
        json!({"code":"000000","isInfavors":[{"contentId":"1","isInfavor":if subscribed {"1"} else {"0"}}]}), profile()]);
    result
}
fn wire(values: &[serde_json::Value]) -> Vec<String> {
    values.iter().enumerate().map(|(i,v)| {
        let body=v.to_string();
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: p{i}\r\nConnection: close\r\n\r\n{body}",body.len())
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
pub(in crate::provider) async fn server(
    responses: Vec<String>,
) -> (MiguProvider, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = vec![];
        for response in responses {
            let request = tokio::time::timeout(Duration::from_secs(10), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![];
                loop {
                    let mut buffer = [0; 1024];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
                String::from_utf8(bytes).unwrap()
            })
            .await
            .unwrap();
            requests.push(request);
        }
        requests
    });
    (
        MiguProvider::from_client(MiguClient::test_client().with_catalog_test_origin(origin)),
        task,
    )
}

#[tokio::test]
async fn favorite_writes_use_one_mutation_full_collection_and_explicit_state_for_each_source() {
    for caller in [false, true] {
        for subscribed in [false, true] {
            let ids: Vec<_> = if subscribed {
                (1..=50).chain([1]).collect()
            } else {
                vec![]
            };
            let values = frames(subscribed, &ids);
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
                .set_track_subscription("1", subscribed, Some(alias))
                .await
                .unwrap();
            assert_eq!(result.resource_ref.to_string(), "migu:1");
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.extensions["favorite_playlist_ref"], "migu:77");
            assert!(
                !serde_json::to_string(&result)
                    .unwrap()
                    .contains("pacmtoken")
            );
            let token = format!("p{}", count - 1);
            if caller {
                let update = p.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    token
                );
                assert_eq!(read(&store, "A"), a);
            } else {
                assert_eq!(read(&store, "A").token(), token);
                assert!(p.take_response_credential().unwrap().is_none());
            }
            assert_eq!(read(&store, "B"), b);
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), count);
            let writes: Vec<_> = requests.iter().filter(|r| r.starts_with("POST ")).collect();
            assert_eq!(writes.len(), 1);
            let (headers, body) = writes[0].split_once("\r\n\r\n").unwrap();
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                body,
                if subscribed {
                    json!({"contentIds":["1"]})
                } else {
                    json!({"channel":"23","contentId":"1","songflag":"2"})
                }
            );
            assert!(headers.starts_with(if subscribed {
                "POST /pc/user/api/add-music-list-song/v1.0 "
            } else {
                "POST /pc/user/h5-import-musiclist/v1.0 "
            }));
            assert!(
                requests[count - 2]
                    .starts_with("GET /pc/v1.0/content/inMusicLists.do?type=1&contentId=1 ")
            );
            for (i, request) in requests.iter().enumerate() {
                let token = if i == 0 {
                    "initial".into()
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
async fn favorite_writes_never_confirm_incomplete_changed_or_unknown_readback_and_keep_verified_updates()
 {
    for caller in [false, true] {
        for variant in 0..8 {
            let mut values = frames(true, &[1, 2, 1]);
            let count = match variant {
                0 => {
                    values[5] = json!({"code":"299999"});
                    6
                }
                1 => {
                    values[18]["isInfavors"] = json!([]);
                    20
                }
                2 => {
                    values[18]["isInfavors"][0]["isInfavor"] = json!("0");
                    20
                }
                3 => {
                    values[18]["isInfavors"][0]["contentId"] = json!("other");
                    20
                }
                4 => {
                    values[12]["data"]["songList"] = json!([]);
                    14
                }
                5 => {
                    values[16] = data(home("88"));
                    18
                }
                6 => {
                    values[5]["userId"] = json!("222");
                    6
                }
                _ => {
                    values[19]["data"]["userId"] = json!("222");
                    20
                }
            };
            values.truncate(count);
            let (mut p, requests) = server(wire(&values)).await;
            let (store, a, b) = setup(&mut p);
            let alias = if caller {
                p = p.caller_scope(&a.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let mut error = p
                .set_track_subscription("1", true, Some(alias))
                .await
                .unwrap_err();
            let auth = variant >= 6;
            assert_eq!(
                error.code,
                if auth {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert!(!error.retryable);
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            let expected = values
                .iter()
                .enumerate()
                .rev()
                .find(|(_, v)| v["data"]["nickName"].is_string())
                .map(|(i, _)| format!("p{i}"));
            let update = error.take_caller_credential_update();
            if caller && !auth {
                assert_eq!(
                    MiguCredential::parse_caller(&update.unwrap())
                        .unwrap()
                        .token(),
                    expected.unwrap()
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
            assert_eq!(
                requests
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| r.starts_with("POST "))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn favorite_writes_reject_invalid_inputs_and_foreign_preflight_owner_without_mutation() {
    let (mut p, requests) = server(vec![]).await;
    let (_, a, _) = setup(&mut p);
    for id in ["", "bad/id", "1,2"] {
        assert_eq!(
            p.set_track_subscription(id, true, Some("A"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        p.set_track_subscription("1", true, Some("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let scoped = p.caller_scope(&a.caller().unwrap()).unwrap();
    assert_eq!(
        scoped
            .set_track_subscription("1", true, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(requests.await.unwrap().is_empty());
    let mut values = frames(true, &[1]);
    values[3]["data"]["ownerId"] = json!("222");
    values.truncate(5);
    let (mut p, requests) = server(wire(&values)).await;
    setup(&mut p);
    let e = p
        .set_track_subscription("1", true, Some("A"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::UpstreamError);
    assert!(e.details.get("write_outcome").is_none());
    assert!(
        !requests
            .await
            .unwrap()
            .iter()
            .any(|r| r.starts_with("POST "))
    );
}

#[tokio::test]
async fn favorite_write_late_responses_at_each_boundary_never_cross_a_replaced_or_removed_account()
{
    let values = frames(true, &[1]);
    let replies = wire(&values);
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
            let task =
                tokio::spawn(
                    async move { worker.set_track_subscription("1", true, Some(alias)).await },
                );
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            let replacement = MiguCredential::verified("111".into(), "new-login".into()).unwrap();
            if caller {
                *p.caller_credential.as_ref().unwrap().lock().unwrap() = replacement.clone();
            } else if stage % 2 == 0 {
                store.remove(Platform::Migu, "A").unwrap();
            } else {
                store.put(&stored("A", &replacement)).unwrap();
            }
            release.send(()).unwrap();
            let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict, "stage {stage}");
            assert!(e.take_caller_credential_update().is_none());
            assert!(p.take_response_credential().unwrap().is_none());
            assert_eq!(e.details.get("write_outcome").is_some(), stage >= 6);
            if caller {
                assert_eq!(read(&store, "A"), a);
            } else if stage % 2 != 0 {
                assert_eq!(read(&store, "A"), replacement);
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

#[tokio::test]
async fn favorite_write_timeouts_do_not_retry_and_export_only_prior_profile_verified_rotations() {
    let values = frames(true, &[1]);
    let replies = wire(&values);
    for stage in 1..=replies.len() {
        let (mut p, seen, release, server) = gated(replies[..stage].to_vec()).await;
        let (_, a, _) = setup(&mut p);
        p.client = p
            .client
            .with_session_test_timeout(Duration::from_millis(300));
        let p = Arc::new(p.caller_scope(&a.caller().unwrap()).unwrap());
        let worker = p.clone();
        let task =
            tokio::spawn(async move { worker.set_track_subscription("1", true, None).await });
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
        assert_eq!(e.details.get("write_outcome").is_some(), stage >= 6);
        if stage >= 6 {
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
