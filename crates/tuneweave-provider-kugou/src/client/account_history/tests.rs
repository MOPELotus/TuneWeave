use super::*;
use crate::{KugouConfig, device::KugouDevice};
use std::{collections::BTreeMap, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const UID: &str = "123456789";

fn info(id: u64) -> WireInfo {
    WireInfo {
        mixsongid: Some(super::super::dto::Number(id)),
        name: format!("Singer {id} - Song {id}"),
        singername: format!("Singer {id}"),
        singerinfo: vec![WireSinger {
            id: Some(super::super::dto::Number(id + 10)),
            name: format!("Singer {id}"),
        }],
        ..Default::default()
    }
}

fn song(id: u64, operation: u64, played_at: u64, play_count: u64) -> WireSong {
    WireSong {
        mxid: super::super::dto::Number(id),
        op: super::super::dto::Number(operation),
        ot: super::super::dto::Number(played_at),
        pc: super::super::dto::Number(play_count),
        info: Some(info(id)),
        osrs: None,
    }
}

fn data(songs: Vec<WireSong>, has_more: u64, bp: Option<&str>) -> HistoryData {
    HistoryData {
        userid: WireScalar::Text(UID.to_owned()),
        bp: bp.map(|value| WireScalar::Text(value.to_owned())),
        has_more: Some(super::super::dto::Number(has_more)),
        songs,
    }
}

fn native_session(client: KugouLoginClient) -> NativeSession {
    NativeSession {
        client,
        device: KugouDevice::default().identity(),
        user_id: UID.to_owned(),
        token: "synthetic-history-token".into(),
        vip_token: None,
        t1: None,
    }
}

fn response(value: Value) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn one_request_server(frame: String) -> (KugouClient, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert!(read > 0);
            request.extend_from_slice(&buffer[..read]);
            assert!(request.len() < 262_144);
            let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let head = std::str::from_utf8(&request[..end]).unwrap();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + content_length {
                break;
            }
        }
        socket.write_all(frame.as_bytes()).await.unwrap();
        let _ = socket.shutdown().await;
        request
    });
    let mut client = KugouClient::new(&KugouConfig::default()).unwrap();
    client.http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    client.login_test_origin = Some(origin);
    (client, task)
}

#[test]
fn standard_history_request_matches_official_query_body_header_and_signature_contract() {
    let session = native_session(KugouLoginClient::Standard);
    let (parameters, body) = history_request(&session, "cursor-1", 1_800_000_000).unwrap();
    assert_eq!(parameters["appid"], session.client.appid().to_string());
    assert_eq!(
        parameters["clientver"],
        session.client.clientver().to_string()
    );
    assert_eq!(parameters["clienttime"], "1800000000");
    assert_eq!(parameters["dfid"], session.device.dfid());
    assert_eq!(parameters["mid"], session.device.mid);
    assert_eq!(parameters["platform"], "1");
    assert_eq!(parameters["uuid"], "-");
    let signature = parameters.get("signature").unwrap();
    let unsigned: BTreeMap<&str, String> = parameters
        .iter()
        .filter(|(key, _)| **key != "signature")
        .map(|(key, value)| (*key, value.clone()))
        .collect();
    assert_eq!(signature, &android_signature(&unsigned, &body));
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        r#"{"userid":123456789,"token":"synthetic-history-token","bp":"cursor-1","source_classify":"app"}"#
    );
}

#[test]
fn account_history_envelope_rejects_foreign_owner_and_non_success_codes() {
    let foreign = serde_json::to_vec(&json!({
        "status":1,
        "error_code":0,
        "data":{"userid":"987654321","has_more":0,"songs":[]}
    }))
    .unwrap();
    let error = parse_page(&foreign, UID).err().unwrap();
    assert_eq!(error.code, ErrorCode::Conflict);

    for code in [20017, 20018] {
        let body = serde_json::to_vec(&json!({"status":0,"error_code":code,"data":null})).unwrap();
        assert_eq!(
            parse_page(&body, UID).err().unwrap().code,
            ErrorCode::AuthenticationRequired
        );
    }
}

#[test]
fn history_rejects_mismatched_song_identity_and_filters_latest_delete_actions() {
    let mut mismatched = song(10, 1, 100, 1);
    mismatched.info.as_mut().unwrap().mixsongid = Some(super::super::dto::Number(11));
    let mut accumulator = HistoryAccumulator::default();
    assert_eq!(
        accumulator
            .accept(data(vec![mismatched], 0, None), UID)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );

    let mut deleted_by_other_device = song(20, 1, 100, 1);
    deleted_by_other_device.osrs = Some(json!({"27":{"op":0,"ot":200,"pc":2}}));
    let mut readded_by_other_device = song(21, 0, 300, 5);
    readded_by_other_device.osrs = Some(json!({"31":{"op":1,"ot":301,"pc":6}}));
    let mut accumulator = HistoryAccumulator::default();
    assert_eq!(
        accumulator
            .accept(
                data(
                    vec![deleted_by_other_device, readded_by_other_device],
                    0,
                    None
                ),
                UID
            )
            .unwrap(),
        PageDecision::Finished
    );
    let snapshot = accumulator.into_snapshot().unwrap();
    assert_eq!(snapshot.items.len(), 1);
    assert_eq!(snapshot.items[0].track.id, "21");
    assert_eq!(snapshot.items[0].play_count, 6);
    assert_eq!(snapshot.items[0].played_at_seconds, 301);
    assert_eq!(snapshot.items[0].device_action_count, 1);
}

#[test]
fn history_cursor_requires_progress_and_stops_only_on_explicit_terminal_page() {
    let mut accumulator = HistoryAccumulator::default();
    assert_eq!(
        accumulator.accept(data(Vec::new(), 0, None), UID).unwrap(),
        PageDecision::Finished
    );
    assert!(accumulator.complete);
    assert_eq!(accumulator.pages, 1);

    let mut accumulator = HistoryAccumulator::default();
    assert_eq!(
        accumulator
            .accept(data(Vec::new(), 1, Some("next")), UID)
            .unwrap(),
        PageDecision::Continue
    );
    assert_eq!(
        accumulator
            .accept(data(Vec::new(), 1, Some("next")), UID)
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );

    let mut accumulator = HistoryAccumulator::default();
    assert_eq!(
        accumulator
            .accept(data(Vec::new(), 1, None), UID)
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
}

#[test]
fn official_thousand_record_cap_returns_an_explicit_incomplete_snapshot() {
    let songs = (1..=HISTORY_LIMIT)
        .map(|id| song(id as u64, 1, 1_700_000_000 - id as u64, 1))
        .collect();
    let mut accumulator = HistoryAccumulator::default();
    assert_eq!(
        accumulator
            .accept(data(songs, 1, Some("next")), UID)
            .unwrap(),
        PageDecision::Finished
    );
    assert!(!accumulator.complete);
    let snapshot = accumulator.into_snapshot().unwrap();
    assert_eq!(snapshot.items.len(), HISTORY_LIMIT);
    assert!(!snapshot.complete);
}

#[tokio::test]
async fn standard_history_request_uses_the_expected_endpoint_and_kg_tid_header() {
    let response = response(json!({
        "status":1,
        "error_code":0,
        "data":{"userid":UID,"has_more":0,"songs":[]}
    }));
    let (client, request_task) = one_request_server(response).await;
    let snapshot = client
        .native_account_history(&native_session(KugouLoginClient::Standard), || Ok(()))
        .await
        .unwrap();
    assert!(snapshot.items.is_empty());
    assert!(snapshot.complete);
    let request = String::from_utf8(request_task.await.unwrap()).unwrap();
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    let request_line = head.lines().next().unwrap();
    assert!(request_line.starts_with("POST /playhistory/v1/get_songs?"));
    assert!(
        head.lines()
            .any(|line| line.eq_ignore_ascii_case("KG-TID: 27"))
    );
    let target = request_line.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("http://localhost{target}")).unwrap();
    let query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(query.get("platform").map(String::as_str), Some("1"));
    assert_eq!(query.get("uuid").map(String::as_str), Some("-"));
    let signature = query.get("signature").unwrap();
    let signed: BTreeMap<&str, String> = query
        .iter()
        .filter(|(key, _)| key.as_str() != "signature")
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    assert_eq!(signature, &android_signature(&signed, body.as_bytes()));
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["userid"].as_u64(), Some(UID.parse().unwrap()));
    assert_eq!(body["token"], "synthetic-history-token");
    assert_eq!(body["bp"], "");
    assert_eq!(body["source_classify"], "app");
}

#[tokio::test]
async fn concept_history_is_unsupported_before_sending_a_history_request() {
    let client = KugouClient::new(&KugouConfig::default()).unwrap();
    let error = client
        .native_account_history(&native_session(KugouLoginClient::Concept), || Ok(()))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
}
