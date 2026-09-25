use super::*;

fn receipt(count: u64) -> Value {
    json!({"userid":111,"total_ver":10,"pre_total_ver":9,"list_count":count})
}
fn initial() -> Vec<Frame> {
    let mut frames = start();
    frames.push(library(vec![row(37), row(3)], 9).into());
    frames
}

#[tokio::test]
async fn concept_single_delete_provider_uses_complete_library_in_both_ownerships() {
    for caller in [false, true] {
        let pages = |rows: Vec<Value>, version: u64| {
            rows.chunks(30)
                .map(|chunk| {
                    let payload = json!({"userid":111,"total_ver":version,
                        "list_count":rows.len(),"info":chunk});
                    Frame::from(reply(payload))
                })
                .collect::<Vec<_>>()
        };
        let after = (1..=31).map(row).collect::<Vec<_>>();
        let mut before = after.clone();
        before.push(row(37));
        let mut frames = start();
        frames.extend(pages(before, 9));
        frames.push(reply(receipt(31)).into());
        frames.extend(pages(after, 10));
        let mut f = server(frames).await;
        let store = store_client(&mut f.provider, KugouLoginClient::Concept);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let result = provider
            .delete_playlists(&delete(&[37], if caller { None } else { Some("A") }))
            .await
            .unwrap();
        assert_eq!(result.playlist_refs[0].id(), REF);
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(result.extensions["total_ver"], 10);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(provider.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 7);
        assert_eq!(body(&all[3])["page"], 2);
        assert_eq!(body(&all[6])["page"], 2);
        assert!(all[4].starts_with("POST /cloudlist.service/v3/delete_list?"));
        assert_eq!(
            body(&all[4]),
            json!({"userid":111,"token":"next","listid":37,"total_ver":9,"type":0})
        );
        assert_eq!(
            all.iter()
                .filter(|r| r.starts_with("POST /cloudlist.service/v3/delete_list?"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn concept_single_delete_provider_rejects_nonordinary_targets_before_write() {
    for case in ["system", "missing_classification", "absent", "wrong_owner"] {
        let mut selected = row(37);
        if case == "system" {
            selected["is_def"] = json!(2);
        }
        if case == "missing_classification" {
            selected.as_object_mut().unwrap().remove("is_def");
        }
        if case == "wrong_owner" {
            selected["list_create_userid"] = json!(222);
        }
        let mut frames = start();
        frames.push(
            library(
                if case == "absent" {
                    vec![row(3)]
                } else {
                    vec![selected, row(3)]
                },
                9,
            )
            .into(),
        );
        let mut f = server(frames).await;
        store_client(&mut f.provider, KugouLoginClient::Concept);
        let e = f
            .provider
            .delete_playlists(&delete(&[37], Some("A")))
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none(), "{case}");
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let mut f = server(vec![]).await;
    store_client(&mut f.provider, KugouLoginClient::Concept);
    for id in ["cloudlist:111:1:37", "cloudlist:222:0:37"] {
        let request = PlaylistDeleteRequest {
            playlist_refs: vec![ResourceRef::new(Platform::Kugou, id).unwrap()],
            account: Some("A".into()),
        };
        assert!(f.provider.delete_playlists(&request).await.is_err());
    }
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_single_delete_provider_requires_exact_removal_and_versioned_readback() {
    for case in [
        "retained",
        "unrelated",
        "missing_other",
        "added",
        "version",
        "count",
        "uid",
    ] {
        let mut after = vec![row(3)];
        let mut ack = receipt(1);
        let mut post_uid = 111;
        match case {
            "retained" => after.push(row(37)),
            "unrelated" => after[0]["name"] = json!("Changed elsewhere"),
            "missing_other" => after.clear(),
            "added" => after.push(row(38)),
            "version" => ack["total_ver"] = json!(11),
            "count" => ack["list_count"] = json!(2),
            _ => post_uid = 222,
        }
        let mut frames = initial();
        frames.push(reply(ack).into());
        frames.push(
            reply(json!({"userid":post_uid,"total_ver":10,"list_count":after.len(),"info":after}))
                .into(),
        );
        let mut f = server(frames).await;
        store_client(&mut f.provider, KugouLoginClient::Concept);
        let e = f
            .provider
            .delete_playlists(&delete(&[37], Some("A")))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict, "{case}");
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert_eq!(
            e.details["unconfirmed_refs"],
            json!(["kugou:cloudlist:111:0:37"])
        );
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn concept_single_delete_provider_rejected_or_malformed_ack_is_unconfirmed_and_not_retried() {
    for caller in [false, true] {
        for response in [
            json!({"status":0,"error_code":20010,"data":"private-response-marker"}),
            json!({"status":0,"error_code":20017}),
            json!({"status":1,"data":{"info":[{"listid":37,"code":1}]}}),
            json!({"status":1,"data":{"userid":222,"total_ver":10,"pre_total_ver":9,"list_count":1}}),
        ] {
            let mut frames = initial();
            frames.push(raw(response).into());
            let mut f = server(frames).await;
            let store = store_client(&mut f.provider, KugouLoginClient::Concept);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut e = provider
                .delete_playlists(&delete(&[37], if caller { None } else { Some("A") }))
                .await
                .unwrap_err();
            assert_eq!(e.details["write_requests_dispatched"], 1);
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            assert!(!format!("{e:?}").contains("private-response-marker"));
            if matches!(
                e.code,
                ErrorCode::AuthenticationRequired | ErrorCode::Conflict
            ) {
                assert!(e.take_caller_credential_update().is_none());
            }
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            assert_eq!(read(&store, "B"), other);
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn concept_single_delete_provider_relogin_discards_late_write_and_readback_results() {
    for caller in [false, true] {
        for readback in [false, true] {
            for failed in [false, true] {
                let mut frames = initial();
                if readback {
                    frames.push(reply(receipt(1)).into());
                }
                let response = if failed {
                    raw(json!({"status":0,"error_code":20017}))
                } else if readback {
                    library(vec![row(3)], 10)
                } else {
                    reply(receipt(1))
                };
                let (frame, resume) = paused(response);
                frames.push(frame);
                let count = frames.len();
                let mut f = server(frames).await;
                let store = store_client(&mut f.provider, KugouLoginClient::Concept);
                let saved = read(&store, "A");
                let other = read(&store, "B");
                let provider = if caller {
                    f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
                } else {
                    f.provider.clone()
                };
                let p = provider.clone();
                let task = tokio::spawn(async move {
                    p.delete_playlists(&delete(&[37], if caller { None } else { Some("A") }))
                        .await
                });
                for _ in 0..count {
                    f.seen.recv().await.unwrap();
                }
                let mut session = saved.native().session.clone();
                session.token = "new-login".into();
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
                assert_eq!(read(&store, "B"), other);
                assert_eq!(f.requests.await.unwrap().len(), count);
            }
        }
    }
}
