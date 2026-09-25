use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const GID: &str = "collection_1_222_88_0";

fn identity(count: usize) -> Value {
    json!({"global_collection_id":GID,"list_create_userid":222,
        "list_create_listid":88,"specialid":900,"source":1,"is_publish":1,"count":count})
}
fn metadata(count: usize) -> Vec<u8> {
    wire(json!([identity(count)]))
}
fn hint(count: usize) -> Vec<u8> {
    let mut value = identity(count);
    value["name"] = json!("Source");
    // The ordinary list ID is deliberately unrelated to specialid.
    value["listid"] = json!(88);
    wire(json!([value]))
}
fn song(id: u64) -> Value {
    json!({"mixsongid":id,"hash":format!("{id:032X}"),"album_id":42,
        "name":format!("Song {id}"),"timelen":123456,"size":1234,"bitrate":128,
        "publish":0})
}
fn page(rows: Vec<Value>, count: usize) -> Value {
    json!({"userid":222,"listid":88,"list_ver":7,"count":count,
        "pagesize":100,"list_info":identity(count),"info":rows})
}
fn wire(value: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"status":1,"error_code":0,"data":value})).unwrap()
}

#[test]
fn concept_collection_plan_empty_still_requires_a_verified_song_page() {
    assert!(
        Preparation::new(&metadata(0), GID)
            .unwrap()
            .finish()
            .is_err()
    );
    let mut read = Preparation::new(&metadata(0), GID).unwrap();
    assert!(read.push(&wire(page(vec![], 0)), 0).unwrap());
    let plan = read.finish().unwrap();
    assert_eq!(plan.occurrence_count, 0);
    assert!(plan.songs.is_empty());
    assert_eq!(plan.source_version, 7);
}

#[test]
fn concept_collection_plan_preserves_first_order_raw_units_and_deduplicates_positive_mix_ids() {
    let mut same_hash_other_mix = song(4);
    same_hash_other_mix["hash"] = song(2)["hash"].clone();
    let mut duplicate = song(2);
    duplicate["name"] = json!("Later name");
    let mut read = Preparation::new(&metadata(4), GID).unwrap();
    assert!(
        read.push(
            &wire(page(
                vec![song(2), song(1), duplicate, same_hash_other_mix],
                4
            )),
            0
        )
        .unwrap()
    );
    let plan = read.finish().unwrap();
    assert_eq!(plan.occurrence_count, 4);
    assert_eq!(
        plan.songs.iter().map(|s| s.mixsongid.0).collect::<Vec<_>>(),
        [2, 1, 4]
    );
    assert_eq!(plan.songs[0].fields["name"], "Song 2");
    assert_eq!(plan.songs[0].fields["timelen"], 123456);
    // Input preparation carries raw facts; publish is not a grant or a filter.
    assert_eq!(plan.songs[0].fields["publish"], 0);
}

#[test]
fn concept_collection_plan_requires_metadata_and_every_page_identity() {
    for key in [
        "global_collection_id",
        "list_create_userid",
        "list_create_listid",
        "specialid",
        "source",
        "is_publish",
        "count",
    ] {
        let mut meta = identity(1);
        meta.as_object_mut().unwrap().remove(key);
        assert!(
            Preparation::new(&wire(json!([meta])), GID).is_err(),
            "{key}"
        );
    }
    for (key, value) in [
        ("global_collection_id", json!("collection_1_333_88_0")),
        ("list_create_userid", json!(333)),
        ("list_create_listid", json!(89)),
        ("specialid", json!(901)),
        ("source", json!(5)),
        ("is_publish", json!(0)),
        ("count", json!(2)),
    ] {
        let mut value_page = page(vec![song(1)], 1);
        value_page["list_info"][key] = value;
        assert!(
            Preparation::new(&metadata(1), GID)
                .unwrap()
                .push(&wire(value_page), 0)
                .is_err(),
            "{key}"
        );
    }
    assert!(Preparation::new(&wire(json!([identity(0), identity(0)])), GID).is_err());
}

#[test]
fn concept_collection_plan_rejects_bad_or_conflicting_song_identities_without_filtering() {
    for (key, value) in [
        ("mixsongid", json!(0)),
        ("mixsongid", json!("01")),
        ("mixsongid", json!(u64::MAX)),
        ("hash", json!("invalid")),
        ("album_id", json!(0)),
        ("album_id", json!("042")),
    ] {
        let mut row = song(1);
        row[key] = value;
        assert!(
            Preparation::new(&metadata(1), GID)
                .unwrap()
                .push(&wire(page(vec![row], 1)), 0)
                .is_err(),
            "{key}"
        );
    }
    for key in ["mixsongid", "hash", "album_id"] {
        let mut row = song(1);
        row.as_object_mut().unwrap().remove(key);
        assert!(
            Preparation::new(&metadata(1), GID)
                .unwrap()
                .push(&wire(page(vec![row], 1)), 0)
                .is_err()
        );
    }
    for key in ["hash", "album_id"] {
        let mut row = song(1);
        row[key] = song(2)[key].clone();
        if key == "album_id" {
            row[key] = json!(43);
        }
        assert!(
            Preparation::new(&metadata(2), GID)
                .unwrap()
                .push(&wire(page(vec![song(1), row], 2)), 0)
                .is_err()
        );
    }
}

#[test]
fn concept_collection_plan_paging_requires_progress_counts_and_stable_version() {
    let first = (1..=100).map(song).collect::<Vec<_>>();
    for (key, value) in [
        ("userid", json!(333)),
        ("listid", json!(89)),
        ("list_ver", json!(8)),
        ("count", json!(102)),
        ("pagesize", json!(99)),
    ] {
        let mut read = Preparation::new(&metadata(101), GID).unwrap();
        assert!(!read.push(&wire(page(first.clone(), 101)), 0).unwrap());
        let mut next = page(vec![song(101)], 101);
        next[key] = value;
        assert!(read.push(&wire(next), 100).is_err(), "{key}");
    }
    for rows in [vec![], vec![song(101), song(102)]] {
        let mut read = Preparation::new(&metadata(101), GID).unwrap();
        read.push(&wire(page(first.clone(), 101)), 0).unwrap();
        assert!(read.push(&wire(page(rows, 101)), 100).is_err());
    }
    let mut read = Preparation::new(&metadata(200), GID).unwrap();
    read.push(&wire(page(first.clone(), 200)), 0).unwrap();
    assert!(read.push(&wire(page(first, 200)), 100).is_err());
    let mut read = Preparation::new(&metadata(1), GID).unwrap();
    assert!(read.push(&wire(page(vec![song(1)], 1)), 100).is_err());
    assert!(
        Preparation::new(&metadata(101), GID)
            .unwrap()
            .finish()
            .is_err()
    );
}

#[test]
fn concept_collection_plan_ignores_unproven_page_echo_and_keeps_cross_page_duplicates() {
    let mut read = Preparation::new(&metadata(101), GID).unwrap();
    let mut first = page((1..=100).map(song).collect(), 101);
    first["page"] = json!(0);
    assert!(!read.push(&wire(first), 0).unwrap());
    let mut last = page(vec![song(1)], 101);
    last["page"] = json!(900);
    assert!(read.push(&wire(last), 100).unwrap());
    let plan = read.finish().unwrap();
    assert_eq!(plan.songs.len(), 100);
    assert_eq!(plan.occurrence_count, 101);
}

#[test]
fn concept_collection_plan_rejects_business_errors_truncation_and_all_budgets() {
    for bytes in [
        br#"{"status":0,"error_code":20017}"#.to_vec(),
        b"{\"status\":1".to_vec(),
        br#"{"status":1,"status":1,"data":[]}"#.to_vec(),
    ] {
        assert!(Preparation::new(&bytes, GID).is_err());
    }
    assert!(Preparation::new(&metadata(MAX_ITEMS + 1), GID).is_err());
    let mut read = Preparation::new(&metadata(1), GID).unwrap();
    read.used = TOTAL_LIMIT;
    assert!(read.push(&wire(page(vec![song(1)], 1)), 0).is_err());
    let mut row = song(1);
    row["name"] = json!("x".repeat(RESPONSE_LIMIT));
    assert!(
        Preparation::new(&metadata(1), GID)
            .unwrap()
            .push(&wire(page(vec![row], 1)), 0)
            .is_err()
    );
}

async fn server(responses: Vec<Vec<u8>>) -> (KugouClient, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for body in responses {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
        requests
    });
    let mut client = KugouClient::new(&KugouConfig::default()).unwrap();
    client.http = reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .build()
        .unwrap();
    client.login_test_origin = Some(origin);
    (client, task)
}

#[tokio::test]
async fn concept_collection_plan_loopback_uses_only_anonymous_read_endpoints_and_exact_signatures()
{
    let (client, requests) = server(vec![
        hint(101),
        metadata(101),
        wire(page((1..=100).map(song).collect(), 101)),
        wire(page(vec![song(1)], 101)),
    ])
    .await;
    let device = crate::device::KugouDevice::default().identity();
    let mut observed = 0;
    let plan = client
        .concept_collection_plan(GID, &device, "111", || {
            observed += 1;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(plan.occurrence_count, 101);
    assert_eq!(plan.songs.len(), 100);
    assert!(observed >= 7);
    let all = requests.await.unwrap();
    assert_eq!(all.len(), 4);
    for (i, request) in all.iter().enumerate() {
        let (head, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(!head.to_ascii_lowercase().contains("cookie:"));
        assert!(!head.to_ascii_lowercase().contains("authorization:"));
        let target = head
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = Url::parse(&format!("http://localhost{target}")).unwrap();
        let mut query = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<BTreeMap<_, _>>();
        let signature = query.remove("signature").unwrap();
        let signed = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        if i == 0 {
            assert_eq!(url.path(), "/v3/get_list_info");
            assert_eq!(
                signature,
                crate::signing::android_signature(&signed, body.as_bytes())
            );
            let data: Value = serde_json::from_str(body).unwrap();
            assert_eq!(data["userid"], 0);
            assert_eq!(data["token"], "");
            continue;
        }
        assert!(!request.contains("token"));
        assert_eq!(signature, concept_signature(&signed, body.as_bytes()));
        assert_eq!(query["clientver"], CLIENT_VERSION);
        assert_eq!(query["appid"], "3116");
        if i == 1 {
            assert_eq!(url.path(), METADATA_PATH);
            assert!(head.starts_with("POST "));
            assert_eq!(
                serde_json::from_str::<Value>(body).unwrap(),
                json!({"data":[{"userid":222,"specialid":900,"global_collection_id":GID}]})
            );
        } else {
            assert_eq!(url.path(), SONGS_PATH);
            assert!(head.starts_with("GET "));
            assert_eq!(query["begin_idx"], ((i - 2) * 100).to_string());
            assert_eq!(query["userid"], "222");
            assert_eq!(query["specialid"], "900");
            assert_eq!(query["type"], "0");
            assert!(!query.contains_key("plat"));
        }
    }
}

#[tokio::test]
async fn concept_collection_plan_loopback_observes_failure_and_stops_before_more_io() {
    let (client, requests) = server(vec![hint(101), metadata(101), b"bad response".to_vec()]).await;
    let device = crate::device::KugouDevice::default().identity();
    let mut observations = 0;
    let error = client
        .concept_collection_plan(GID, &device, "111", || {
            observations += 1;
            if observations >= 6 {
                Err(changed())
            } else {
                Ok(())
            }
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn concept_collection_plan_bootstrap_requires_exact_source_identity_before_v1() {
    for key in ["list_create_userid", "specialid", "list_create_listid"] {
        let mut data: Value = serde_json::from_slice(&hint(0)).unwrap();
        data["data"][0].as_object_mut().unwrap().remove(key);
        let (client, requests) = server(vec![serde_json::to_vec(&data).unwrap()]).await;
        assert!(
            client
                .concept_collection_plan(
                    GID,
                    &crate::device::KugouDevice::default().identity(),
                    "111",
                    || Ok(())
                )
                .await
                .is_err()
        );
        assert_eq!(requests.await.unwrap().len(), 1);
    }
    let (client, requests) = server(vec![hint(0)]).await;
    assert_eq!(
        client
            .concept_collection_plan(
                GID,
                &crate::device::KugouDevice::default().identity(),
                "222",
                || Ok(())
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(requests.await.unwrap().len(), 1);
    let mut meta = identity(0);
    meta["specialid"] = json!(901);
    let (client, requests) = server(vec![hint(0), wire(json!([meta]))]).await;
    assert!(
        client
            .concept_collection_plan(
                GID,
                &crate::device::KugouDevice::default().identity(),
                "111",
                || Ok(())
            )
            .await
            .is_err()
    );
    assert_eq!(requests.await.unwrap().len(), 2);
}
