use super::*;
use crate::account::cloud::tests::{
    Frame as BinaryFrame, encrypted, plaintext, server as binary_server,
};

fn binary_start() -> Vec<BinaryFrame> {
    vec![
        exchange("111", "next").into(),
        profile("111").into(),
        library(vec![row(37), row(38)], 9).into(),
    ]
}
fn receipt() -> Value {
    json!({"userid":111,"total_ver":10,"pre_total_ver":9,"list_count":2,
        "info":{"code":1,"listid":37,"type":0,"name":"Renamed","sort":42}})
}
fn changed() -> Value {
    let mut r = row(37);
    r["name"] = json!("Renamed");
    r
}

#[tokio::test]
async fn concept_metadata_provider_uses_v1_with_complete_readback_in_both_ownerships() {
    for caller in [false, true] {
        for clear in [false, true] {
            let mut updated = changed();
            if clear {
                updated["intro"] = json!("");
                updated["tags"] = json!("");
            }
            let mut frames = binary_start();
            frames.push(encrypted(receipt()));
            frames.push(library(vec![updated, row(38)], 10).into());
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
            let mut request = update(if caller { None } else { Some("A") });
            if clear {
                request.description = Some(String::new());
                request.tags = Some(vec![]);
            }
            let result = provider.update_playlist(REF, &request).await.unwrap();
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            let playlist = result.playlist.unwrap();
            assert_eq!(playlist.name, "Renamed");
            assert_eq!(playlist.extensions["is_private"], true);
            assert_eq!(
                playlist.description,
                if clear { "" } else { "  Preserve intro\n" }
            );
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
                assert!(provider.take_response_credential().unwrap().is_some());
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), 5);
            assert!(
                requests[3]
                    .head
                    .starts_with("POST /cloudlist.service/v1/modify_list?")
            );
            let body = plaintext(&requests[3]);
            assert!(body.get("is_pri").is_none());
            assert!(body.get("token").is_none());
            assert_eq!(body["name"], "Renamed");
            assert_eq!(body["sort"], 42);
            assert_eq!(body["intro"], if clear { "" } else { "  Preserve intro\n" });
            assert_eq!(body["tags"], if clear { "" } else { "甲,乙" });
        }
    }
}

#[tokio::test]
async fn concept_metadata_provider_noop_keeps_existing_snapshot_without_write() {
    let f = binary_server(binary_start()).await;
    let mut provider = KugouProvider::from_client(f.client);
    store_client(&mut provider, KugouLoginClient::Concept);
    let mut request = update(Some("A"));
    request.name = Some("Name 37".into());
    let result = provider.update_playlist(REF, &request).await.unwrap();
    assert_eq!(result.extensions["changed"], false);
    assert_eq!(result.extensions["write_requests_dispatched"], 0);
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn concept_metadata_provider_rejects_unapplied_changes_and_edited_other_fields() {
    for case in ["name", "intro", "privacy", "unrelated", "ack_version"] {
        let mut selected = changed();
        let mut other = row(38);
        let mut ack = receipt();
        match case {
            "name" => selected["name"] = json!("Name 37"),
            "intro" => selected["intro"] = json!("Changed without request"),
            "privacy" => selected["is_pri"] = json!(0),
            "unrelated" => other["name"] = json!("Changed other list"),
            _ => ack["total_ver"] = json!(11),
        }
        let mut frames = binary_start();
        frames.push(encrypted(ack));
        frames.push(library(vec![selected, other], 10).into());
        let f = binary_server(frames).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_client(&mut provider, KugouLoginClient::Concept);
        let error = provider
            .update_playlist(REF, &update(Some("A")))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert!(!error.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn concept_metadata_provider_relogin_discards_late_write_success_and_failure() {
    for caller in [false, true] {
        for failed in [false, true] {
            let mut frames = binary_start();
            let mut frame = if failed {
                BinaryFrame::from(raw(json!({"status":0,"error_code":20017})))
            } else {
                encrypted(receipt())
            };
            let (resume, gate) = tokio::sync::oneshot::channel();
            frame.gate = Some(gate);
            frames.push(frame);
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
                p.update_playlist(REF, &update(if caller { None } else { Some("A") }))
                    .await
            });
            for _ in 0..4 {
                f.seen.recv().await.unwrap();
            }
            let mut session = read(&store, "A").native().session.clone();
            session.token = "new-login".into();
            let replacement = KugouCredential::verified(session).unwrap();
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                    Some(replacement.clone());
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            assert!(error.take_caller_credential_update().is_none());
            assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}
