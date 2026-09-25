use super::*;

fn inputs(count: usize) -> Vec<TrackInput> {
    (0..count)
        .map(|index| {
            let mut value = input();
            let TrackWire::Standard(row) = &mut value.wire else {
                unreachable!()
            };
            row.mixsongid += index as u64;
            row.hash = format!("{index:032x}");
            row.album_id = (42 + index).to_string();
            value
        })
        .collect()
}

fn batch_receipt(count: usize) -> Value {
    json!({"userid":123456789,"listid":37,"list_ver":8,"pre_list_ver":7,"count":count+1,
        "info":(0..count).rev().map(|i| json!({"fileid":99+i,"name":"Song.mp3","sort":i+1,
        "hash":format!("{i:032x}"),"album_id":(42+i).to_string(),"mixsongid":900+i,"code":1})).collect::<Vec<_>>()})
}

#[tokio::test]
async fn standard_add_batch_wire_uses_one_request_and_official_ordinal_order() {
    for count in [2, 100] {
        let (client, requests) = server(vec![ok(batch_receipt(count))]).await;
        let source = session(KugouLoginClient::Standard);
        let (ack, items) = client
            .native_add_standard_tracks(&source, 37, 7, &inputs(count))
            .await
            .unwrap();
        assert_eq!(ack.version, Some(8));
        assert_eq!(ack.count, Some(count as u64 + 1));
        assert_eq!(
            items.iter().map(|item| item.mixsongid).collect::<Vec<_>>(),
            (900..900 + count as u64).collect::<Vec<_>>()
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
        assert_eq!(signature, android_signature(&query, body.as_bytes()));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["list_ver"], 7);
        assert_eq!(body["userid"], "123456789");
        assert_eq!(body["allow_part_fail"], 1);
        let rows = body["data"].as_array().unwrap();
        assert_eq!(rows.len(), count);
        for (wire_index, row) in rows.iter().enumerate() {
            let ordinal = count - wire_index - 1;
            assert_eq!(row["mixsongid"], 900 + ordinal);
            assert_eq!(row["sort"], ordinal);
            assert_eq!(row["album_id"], (42 + ordinal).to_string());
            assert_eq!(row["number"], 1);
        }
    }
}

#[test]
fn standard_add_batch_ack_rejects_partial_duplicate_and_foreign_items() {
    let values = inputs(2);
    let inputs = values
        .iter()
        .map(|value| {
            let TrackWire::Standard(row) = &value.wire else {
                unreachable!()
            };
            row
        })
        .collect::<Vec<_>>();
    for case in [
        "missing",
        "extra",
        "duplicate_mix",
        "duplicate_file",
        "foreign_mix",
        "hash",
        "album",
        "failed",
    ] {
        let mut data = batch_receipt(2);
        match case {
            "missing" => {
                data["info"].as_array_mut().unwrap().pop();
            }
            "extra" => {
                let row = data["info"][0].clone();
                data["info"].as_array_mut().unwrap().push(row);
            }
            "duplicate_mix" => data["info"][0] = data["info"][1].clone(),
            "duplicate_file" => data["info"][0]["fileid"] = data["info"][1]["fileid"].clone(),
            "foreign_mix" => data["info"][0]["mixsongid"] = json!(902),
            "hash" => data["info"][0]["hash"] = json!("f".repeat(32)),
            "album" => data["info"][0]["album_id"] = json!(999),
            _ => data["info"][0]["code"] = json!(205),
        }
        let bytes = serde_json::to_vec(&json!({"status":1,"data":data})).unwrap();
        assert!(
            acknowledge(&bytes, "123456789", 37, 7, &inputs).is_err(),
            "{case}"
        );
    }
}

#[tokio::test]
async fn standard_add_batch_rejects_duplicates_limits_and_profiles_before_io() {
    let (client, requests) = server(vec![]).await;
    let source = session(KugouLoginClient::Standard);
    for values in [inputs(0), inputs(101), vec![input(), input()]] {
        assert!(
            client
                .native_add_standard_tracks(&source, 37, 7, &values)
                .await
                .is_err()
        );
    }
    for profile in [KugouLoginClient::Concept, KugouLoginClient::Web] {
        assert!(
            client
                .native_add_standard_tracks(&session(profile), 37, 7, &inputs(2))
                .await
                .is_err()
        );
    }
    assert!(requests.await.unwrap().is_empty());
}
