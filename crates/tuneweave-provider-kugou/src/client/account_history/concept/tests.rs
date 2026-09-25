use super::*;
use crate::account::cloud::tests::{binary, server};

fn session(client: KugouLoginClient) -> NativeSession {
    NativeSession {
        client,
        device: crate::device::KugouDevice::default().identity(),
        user_id: "123456789".into(),
        token: "synthetic-history-token".into(),
        vip_token: None,
        t1: None,
    }
}
fn raw(body: Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn reply(data: Value) -> String {
    raw(json!({"status":1,"error_code":0,"data":data}))
}

fn song(id: u64, ot: u64, op: u64, pc: u64) -> Value {
    json!({"mxid":id,"ot":ot,"op":op,"pc":pc,"info":{"mixsongid":id,"name":format!("Song {id}"),"singername":"Singer","timelen":180000,"album_id":30,"albuminfo":{"id":30,"name":"Album"}}})
}
fn data(songs: Vec<Value>, more: u64, bp: &str) -> Value {
    json!({"userid":"123456789","songs":songs,"has_more":more,"bp":bp})
}
fn parsed(value: Value) -> Data {
    parse(
        &serde_json::to_vec(&json!({"status":1,"error_code":0,"data":value})).unwrap(),
        "123456789",
    )
    .unwrap()
}

#[tokio::test]
async fn concept_history_get_signature_cursor_and_main_device_actions_match_official_consumer() {
    let mut first = song(1, 100, 1, 3);
    first["osrs"] = json!({"27":{"op":0,"ot":999,"pc":99}});
    let mut equal_time = song(1, 100, 0, 90);
    equal_time["info"]["name"] = json!("Updated metadata");
    let frames = vec![
        reply(data(vec![first, song(2, 90, 1, 1)], 1, "cursor & 1")).into(),
        reply(data(
            vec![equal_time, song(2, 200, 0, 2), song(3, 180, 1, 4)],
            0,
            "",
        ))
        .into(),
    ];
    let f = server(frames).await;
    let source = session(KugouLoginClient::Concept);
    let mut checks = 0;
    let snapshot = f
        .client
        .concept_account_history(&source, || {
            checks += 1;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(checks, 4);
    assert!(snapshot.complete);
    assert_eq!(snapshot.pages, 2);
    assert_eq!(
        snapshot
            .items
            .iter()
            .map(|s| s.track.id.as_str())
            .collect::<Vec<_>>(),
        ["3", "1"]
    );
    assert_eq!(snapshot.items[1].play_count, 3);
    assert_eq!(snapshot.items[1].played_at_seconds, 100);
    assert_eq!(snapshot.items[1].track.name, "Updated metadata");
    assert_eq!(snapshot.items[1].device_action_count, 0);
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    for (i, r) in requests.iter().enumerate() {
        assert!(r.head.starts_with("GET /playhistory/youth/v1/get_songs?"));
        assert!(r.body.is_empty());
        assert!(!r.head.to_ascii_lowercase().contains("cookie:"));
        let url = Url::parse(&format!(
            "http://localhost{}",
            r.head
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
        ))
        .unwrap();
        let mut q = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(q.len(), 11);
        assert_eq!(q["userid"], source.user_id);
        assert_eq!(q["token"], source.token);
        assert_eq!(q["type"], "1");
        assert_eq!(q["bp"], if i == 0 { "" } else { "cursor & 1" });
        assert_eq!(q["appid"], "3116");
        assert_eq!(q["uuid"], "-");
        assert!(!q.contains_key("source_classify"));
        assert!(!q.contains_key("platform"));
        let sig = q.remove("signature").unwrap();
        let pairs = q.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        assert_eq!(sig, concept_signature(&pairs, b""));
    }
}

#[test]
fn concept_history_rejects_identity_cursor_and_complete_read_budget_failures() {
    for case in [
        "foreign_uid",
        "missing_songs",
        "missing_flag",
        "string_cursor",
    ] {
        let mut v = data(vec![], 0, "");
        match case {
            "foreign_uid" => v["userid"] = json!(222),
            "missing_songs" => {
                v.as_object_mut().unwrap().remove("songs");
            }
            "missing_flag" => {
                v.as_object_mut().unwrap().remove("has_more");
            }
            _ => v["bp"] = json!(1),
        }
        assert!(
            parse(
                &serde_json::to_vec(&json!({"status":1,"error_code":0,"data":v})).unwrap(),
                "123456789"
            )
            .is_err(),
            "{case}"
        );
    }
    for value in [
        data(vec![], 1, ""),
        data(vec![], 1, "invalid\ncursor"),
        data(vec![], 2, "next"),
    ] {
        assert!(
            Accumulator::default()
                .accept(parsed(value), "123456789")
                .is_err()
        );
    }
    let mut wrong = song(1, 1, 1, 1);
    wrong["info"]["mixsongid"] = json!(2);
    let mut acc = Accumulator::default();
    assert_eq!(
        acc.accept(parsed(data(vec![wrong], 0, "")), "123456789")
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let mut acc = Accumulator::default();
    assert!(
        !acc.accept(parsed(data(vec![], 1, "a")), "123456789")
            .unwrap()
    );
    assert!(
        !acc.accept(parsed(data(vec![], 1, "b")), "123456789")
            .unwrap()
    );
    assert!(
        acc.accept(parsed(data(vec![], 1, "a")), "123456789")
            .is_err()
    );
    let mut acc = Accumulator::default();
    assert!(
        acc.accept(
            parsed(data((1..=1001).map(|i| song(i, 1, 1, 1)).collect(), 0, "")),
            "123456789"
        )
        .is_err()
    );
    let mut acc = Accumulator::default();
    for i in 1..PAGE_LIMIT {
        assert!(
            !acc.accept(parsed(data(vec![], 1, &i.to_string())), "123456789")
                .unwrap()
        );
    }
    assert!(
        acc.accept(parsed(data(vec![], 1, "last")), "123456789")
            .is_err()
    );
    let mut acc = Accumulator::default();
    assert!(
        acc.accept(parsed(data(vec![], 0, "")), "123456789")
            .unwrap()
    );
    assert!(acc.finish().unwrap().items.is_empty());
}

#[tokio::test]
async fn concept_history_transport_errors_and_invalid_sessions_never_fall_back() {
    let source = session(KugouLoginClient::Concept);
    let f = server(vec![]).await;
    assert!(
        f.client
            .concept_account_history(&session(KugouLoginClient::Standard), || Ok(()))
            .await
            .is_err()
    );
    assert!(f.requests.await.unwrap().is_empty());
    for frame in [
        binary(302, "application/json", vec![]),
        binary(200, "text/html", vec![]),
        binary(200, "application/json", vec![0; HISTORY_RESPONSE_LIMIT + 1]),
        raw(json!({"status":0,"error_code":20017,"message":"secret-marker"})).into(),
        raw(json!({"status":0,"error_code":123,"message":"secret-marker"})).into(),
    ] {
        let f = server(vec![frame]).await;
        let e = f
            .client
            .concept_account_history(&source, || Ok(()))
            .await
            .unwrap_err();
        assert!(!format!("{e:?}").contains("secret-marker"));
        assert!(!format!("{e:?}").contains(&source.token));
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
    let f = server(vec![reply(data(vec![], 1, "next")).into()]).await;
    let mut n = 0;
    let e = f
        .client
        .concept_account_history(&source, || {
            n += 1;
            if n == 3 {
                Err(identity_conflict())
            } else {
                Ok(())
            }
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn concept_history_total_deadline_byte_budget_and_cancellation_stop_pagination() {
    let source = session(KugouLoginClient::Concept);
    let mut f = server(vec![reply(data(vec![], 0, "")).into()]).await;
    assert!(
        f.client
            .concept_history_with_budget(
                &source,
                || Ok(()),
                tokio::time::Instant::now() + DEADLINE,
                1
            )
            .await
            .is_err()
    );
    assert_eq!(f.requests.await.unwrap().len(), 1);
    for cancel in [false, true] {
        let mut frame = crate::account::cloud::tests::Frame::from(reply(data(vec![], 1, "next")));
        let (resume, gate) = tokio::sync::oneshot::channel();
        frame.gate = Some(gate);
        f = server(vec![frame]).await;
        let client = f.client.clone();
        let source = source.clone();
        let task = tokio::spawn(async move {
            client
                .concept_history_with_budget(
                    &source,
                    || Ok(()),
                    tokio::time::Instant::now() + Duration::from_secs(1),
                    BYTE_LIMIT,
                )
                .await
        });
        f.seen.recv().await.unwrap();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            assert!(task.await.unwrap().is_err());
        }
        resume.send(()).unwrap();
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}
