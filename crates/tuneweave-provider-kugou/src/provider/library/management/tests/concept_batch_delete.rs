use super::*;
use crate::account::cloud::tests::{
    Frame as BinaryFrame, encrypted, plaintext, server as binary_server,
};
use base64::engine::general_purpose::STANDARD as BASE64;

fn receipt() -> Value {
    json!({"userid":111,"pre_total_ver":9,"total_ver":10,"list_count":1,
        "info":[{"code":1,"listid":38},{"code":1,"listid":37,"type":0}]})
}
fn initial() -> Vec<BinaryFrame> {
    vec![
        exchange("111", "next").into(),
        profile("111").into(),
        library(vec![row(37), row(3), row(38)], 9).into(),
    ]
}

#[tokio::test]
async fn concept_batch_delete_provider_verifies_complete_pages_for_both_credential_owners() {
    for caller in [false, true] {
        let pages = |rows: Vec<Value>, version: u64| {
            rows.chunks(30).map(|chunk| BinaryFrame::from(reply(json!({"userid":111,"total_ver":version,"list_count":rows.len(),"info":chunk})))).collect::<Vec<_>>()
        };
        let mut before = (1..=33).map(row).collect::<Vec<_>>();
        // Keep unrelated collected rows and each category's order as well.
        before.push(collection(80));
        let after = before
            .iter()
            .filter(|p| p["listid"] != 1 && p["listid"] != 33)
            .cloned()
            .collect::<Vec<_>>();
        let mut ack = receipt();
        ack["list_count"] = json!(32);
        ack["info"] = json!([{"code":1,"listid":33},{"code":1,"listid":1}]);
        let mut frames = vec![exchange("111", "next").into(), profile("111").into()];
        frames.extend(pages(before, 9));
        frames.push(encrypted(ack));
        frames.extend(pages(after, 10));
        let f = binary_server(frames).await;
        let mut owner = KugouProvider::from_client(f.client);
        let store = store_client(&mut owner, KugouLoginClient::Concept);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            owner.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            owner
        };
        let result = provider
            .delete_playlists(&delete(&[1, 33], if caller { None } else { Some("A") }))
            .await
            .unwrap();
        assert_eq!(result.playlist_refs, delete(&[1, 33], None).playlist_refs);
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(result.extensions["atomic"], false);
        assert_eq!(result.extensions["total_ver"], 10);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(provider.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 7);
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[3].body).unwrap()["page"],
            2
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[6].body).unwrap()["page"],
            2
        );
        assert!(
            requests[4]
                .head
                .starts_with("POST /cloudlist.service/v2/delete_multy_list?")
        );
        let mut decoded = requests[4].clone();
        decoded.body = BASE64.decode(&decoded.body).unwrap();
        assert_eq!(
            plaintext(&decoded),
            json!({"total_ver":9,"data":[{"listid":1,"type":0},{"listid":33,"type":0}]})
        );
    }
}

#[tokio::test]
async fn concept_batch_delete_provider_preflights_every_target_before_one_write() {
    for case in [
        "missing",
        "default",
        "collected",
        "wrong_owner",
        "missing_classification",
    ] {
        let mut target = row(38);
        match case {
            "default" => target["is_def"] = json!(2),
            "collected" => target = collection(38),
            "wrong_owner" => target["list_create_userid"] = json!(222),
            "missing_classification" => {
                target.as_object_mut().unwrap().remove("is_def");
            }
            _ => {}
        }
        let rows = if case == "missing" {
            vec![row(37)]
        } else {
            vec![row(37), target]
        };
        let frames = vec![
            exchange("111", "next").into(),
            profile("111").into(),
            library(rows, 9).into(),
        ];
        let f = binary_server(frames).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_client(&mut provider, KugouLoginClient::Concept);
        let e = provider
            .delete_playlists(&delete(&[37, 38], Some("A")))
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none(), "{case}");
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let f = binary_server(vec![]).await;
    let mut provider = KugouProvider::from_client(f.client);
    store_client(&mut provider, KugouLoginClient::Concept);
    for ids in [vec![], vec![37, 37], (1..=101).collect()] {
        assert!(
            provider
                .delete_playlists(&delete(&ids, Some("A")))
                .await
                .is_err()
        );
    }
    for reference in ["cloudlist:222:0:38", "cloudlist:111:1:38"] {
        let mut request = delete(&[37, 38], Some("A"));
        request.playlist_refs[1] = ResourceRef::new(Platform::Kugou, reference).unwrap();
        assert!(provider.delete_playlists(&request).await.is_err());
    }
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_batch_delete_provider_rejects_partial_receipts_without_retry_or_false_confirmation()
 {
    for case in [
        "partial",
        "omitted",
        "extra",
        "duplicate",
        "wrong_uid",
        "rejected",
    ] {
        let mut ack = receipt();
        match case {
            "partial" => ack["info"][0]["code"] = json!(0),
            "omitted" => {
                ack["info"].as_array_mut().unwrap().pop();
            }
            "extra" => ack["info"]
                .as_array_mut()
                .unwrap()
                .push(json!({"listid":39,"code":1})),
            "duplicate" => ack["info"][0]["listid"] = json!(37),
            "wrong_uid" => ack["userid"] = json!(222),
            _ => {}
        }
        let mut frames = initial();
        frames.push(if case == "rejected" {
            raw(json!({"status":0,"error_code":20017})).into()
        } else {
            encrypted(ack)
        });
        let f = binary_server(frames).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_client(&mut provider, KugouLoginClient::Concept);
        let e = provider
            .delete_playlists(&delete(&[37, 38], Some("A")))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed", "{case}");
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(e.details["confirmed_refs"], json!([]));
        assert_eq!(e.details["not_attempted_refs"], json!([]));
        assert_eq!(
            e.details["unconfirmed_refs"],
            json!(["kugou:cloudlist:111:0:37", "kugou:cloudlist:111:0:38"])
        );
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn concept_batch_delete_provider_requires_exact_complete_postwrite_state() {
    for case in [
        "retained",
        "other_changed",
        "other_missing",
        "new",
        "count",
        "version",
        "reordered",
    ] {
        let mut frames = initial();
        let mut after = vec![row(3)];
        let mut ack = receipt();
        match case {
            "retained" => after.push(row(38)),
            "other_changed" => after[0]["name"] = json!("Changed"),
            "other_missing" => after.clear(),
            "new" => after.push(row(99)),
            "count" => ack["list_count"] = json!(2),
            "version" => ack["total_ver"] = json!(11),
            _ => {
                frames[2] = library(vec![row(37), row(3), row(4), row(38)], 9).into();
                after = vec![row(4), row(3)];
                ack["list_count"] = json!(2);
            }
        }
        frames.push(encrypted(ack));
        frames.push(library(after, 10).into());
        let f = binary_server(frames).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_client(&mut provider, KugouLoginClient::Concept);
        let e = provider
            .delete_playlists(&delete(&[37, 38], Some("A")))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict, "{case}");
        assert_eq!(e.details["confirmed_refs"], json!([]));
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn concept_batch_delete_provider_discards_late_write_and_readback_on_relogin() {
    for caller in [false, true] {
        for boundary in ["write", "readback"] {
            for failed in [false, true] {
                let mut frames = initial();
                let mut paused = if failed {
                    BinaryFrame::from(raw(json!({"status":0,"error_code":20017})))
                } else if boundary == "write" {
                    encrypted(receipt())
                } else {
                    library(vec![row(3)], 10).into()
                };
                let (resume, gate) = tokio::sync::oneshot::channel();
                paused.gate = Some(gate);
                if boundary == "readback" {
                    frames.push(encrypted(receipt()));
                }
                frames.push(paused);
                let mut f = binary_server(frames).await;
                let mut owner = KugouProvider::from_client(f.client);
                let store = store_client(&mut owner, KugouLoginClient::Concept);
                let saved = read(&store, "A");
                let provider = if caller {
                    owner.caller_scope(&saved.caller().unwrap()).unwrap()
                } else {
                    owner
                };
                let p = provider.clone();
                let task = tokio::spawn(async move {
                    p.delete_playlists(&delete(&[37, 38], if caller { None } else { Some("A") }))
                        .await
                });
                let count = if boundary == "write" { 4 } else { 5 };
                for _ in 0..count {
                    f.seen.recv().await.unwrap();
                }
                let mut session = read(&store, "A").native().session.clone();
                session.token = "replacement-login".into();
                let replacement = KugouCredential::verified(session).unwrap();
                if caller {
                    *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                        Some(replacement.clone());
                } else {
                    store.put(&replacement.stored("A").unwrap()).unwrap();
                }
                resume.send(()).unwrap();
                let mut e = task.await.unwrap().unwrap_err();
                assert_eq!(e.code, ErrorCode::Conflict);
                assert_eq!(e.details["write_outcome"], "unconfirmed");
                assert!(e.take_caller_credential_update().is_none());
                assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
                assert_eq!(f.requests.await.unwrap().len(), count);
            }
        }
    }
}
