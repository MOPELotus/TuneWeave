use super::*;

fn inputs() -> Vec<ConceptAlbumTrack> {
    let first = input();
    let mut second = input();
    second.mixsongid = 901;
    second.hash = "b".repeat(32);
    vec![first, second]
}
fn batch_receipt() -> Value {
    let mut data = receipt();
    data["count"] = json!(3);
    let mut second = data["info"][0].clone();
    second["fileid"] = json!(100);
    second["sort"] = json!(1);
    second["mixsongid"] = json!(901);
    second["hash"] = json!("B".repeat(32));
    data["info"].as_array_mut().unwrap().insert(0, second);
    data
}
fn bytes(data: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"status":1,"data":data})).unwrap()
}

#[tokio::test]
async fn concept_batch_add_wire_reverses_items_preserving_ordinals_and_signature() {
    let (client, requests) = server(vec![ok(batch_receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let (version, items) = client
        .native_add_concept_tracks(&source, 37, 7, &inputs())
        .await
        .unwrap();
    assert_eq!(version.count, Some(3));
    assert_eq!(
        items
            .iter()
            .map(|v| (v.file_id, v.sort))
            .collect::<Vec<_>>(),
        [(99, 0), (100, 1)]
    );
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let (head, body) = requests[0].split_once("\r\n\r\n").unwrap();
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    assert_eq!(url.path(), PATH);
    let mut query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    let signature = query.remove("signature").unwrap();
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, concept_signature(&query, body.as_bytes()));
    let wire: Value = serde_json::from_str(body).unwrap();
    assert_eq!(wire["userid"], "123456789");
    assert_eq!(wire["list_ver"], 7);
    assert_eq!(wire["type"], 0);
    assert_eq!(wire["data"][0]["mixsongid"], 901);
    assert_eq!(wire["data"][0]["sort"], 1);
    assert_eq!(wire["data"][1]["mixsongid"], 900);
    assert_eq!(wire["data"][1]["sort"], 0);
    assert!(
        wire["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["number"] == 1)
    );
    assert!(!head.contains(&source.token));
}

#[test]
fn concept_batch_add_ack_matches_exact_identities_independently_of_array_order() {
    for reverse in [false, true] {
        let mut data = batch_receipt();
        if reverse {
            data["info"].as_array_mut().unwrap().reverse();
        }
        let (_, items) = acknowledge_many(&bytes(data), "123456789", 37, 7, &inputs()).unwrap();
        assert_eq!(
            items.iter().map(|v| v.file_id).collect::<Vec<_>>(),
            [99, 100]
        );
    }
    for case in [
        "missing",
        "extra",
        "duplicate_id",
        "duplicate_file",
        "duplicate_sort",
        "hash",
        "album",
        "capacity",
        "unknown_code",
        "cloud",
        "uid",
        "version",
    ] {
        let mut data = batch_receipt();
        match case {
            "missing" => {
                data["info"].as_array_mut().unwrap().pop();
            }
            "extra" => {
                let row = data["info"][0].clone();
                data["info"].as_array_mut().unwrap().push(row);
            }
            "duplicate_id" => data["info"][0]["mixsongid"] = json!(900),
            "duplicate_file" => data["info"][0]["fileid"] = json!(99),
            "duplicate_sort" => data["info"][0]["sort"] = json!(0),
            "hash" => data["info"][0]["hash"] = json!("a".repeat(32)),
            "album" => data["info"][0]["album_id"] = json!(43),
            "capacity" => data["info"][0]["code"] = json!(205),
            "unknown_code" => data["info"][0]["code"] = json!(1),
            "cloud" => data["info"][0]["csong"] = json!(1),
            "uid" => data["userid"] = json!(222),
            _ => data["pre_list_ver"] = json!(6),
        }
        assert!(
            acknowledge_many(&bytes(data), "123456789", 37, 7, &inputs()).is_err(),
            "{case}"
        );
    }
}

#[tokio::test]
async fn concept_batch_add_rejects_ambiguous_inputs_and_more_than_one_segment_before_io() {
    let (client, requests) = server(vec![]).await;
    let source = session(KugouLoginClient::Concept);
    for case in ["empty", "duplicate_id", "duplicate_hash", "over_budget"] {
        let mut values = inputs();
        match case {
            "empty" => values.clear(),
            "duplicate_id" => values[1].mixsongid = 900,
            "duplicate_hash" => values[1].hash = "A".repeat(32),
            _ => {
                values = (0..101)
                    .map(|n| {
                        let mut v = input();
                        v.mixsongid = 1000 + n;
                        v.hash = format!("{n:032x}");
                        v
                    })
                    .collect()
            }
        }
        assert!(
            client
                .native_add_concept_tracks(&source, 37, 7, &values)
                .await
                .is_err(),
            "{case}"
        );
    }
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_batch_add_wire_accepts_one_hundred_items_without_splitting() {
    let values = (0..100)
        .map(|index| {
            let mut value = input();
            value.mixsongid = 1000 + index;
            value.hash = format!("{index:032x}");
            value
        })
        .collect::<Vec<_>>();
    let mut data = receipt();
    data["count"] = json!(101);
    data["info"] = json!(
        values
            .iter()
            .enumerate()
            .rev()
            .map(|(index, input)| json!({
                "fileid":100+index,"name":input.name,"sort":index,"hash":input.hash,
                "album_id":input.album_id,"mixsongid":input.mixsongid
            }))
            .collect::<Vec<_>>()
    );
    let (client, requests) = server(vec![ok(data)]).await;
    let (_, items) = client
        .native_add_concept_tracks(&session(KugouLoginClient::Concept), 37, 7, &values)
        .await
        .unwrap();
    assert_eq!(items.len(), 100);
    assert_eq!(items.first().unwrap().file_id, 100);
    assert_eq!(items.last().unwrap().file_id, 199);
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let (_, body) = requests[0].split_once("\r\n\r\n").unwrap();
    let wire: Value = serde_json::from_str(body).unwrap();
    assert_eq!(wire["data"].as_array().unwrap().len(), 100);
    assert_eq!(wire["data"][0]["sort"], 99);
    assert_eq!(wire["data"][99]["sort"], 0);
}
