use super::*;

const COLLECTED: &str = "cloudlist:111:1:37";
fn receipt(count: u64) -> Value {
    json!({"userid":111,"total_ver":10,"pre_total_ver":9,"list_count":count})
}
fn initial() -> Vec<Frame> {
    let mut frames = start();
    frames.push(library(vec![row(3), collection(37)], 9).into());
    frames
}

#[tokio::test]
async fn concept_new_collection_provider_rejects_before_public_lookup_or_write_for_both_owners() {
    for caller in [false, true] {
        let rows = (1..=31).map(row).collect::<Vec<_>>();
        let mut frames = start();
        for page in rows.chunks(30) {
            frames.push(
                reply(json!({"userid":111,"total_ver":9,"list_count":31,"info":page})).into(),
            );
        }
        let mut f = server(frames).await;
        let store = store_client(&mut f.provider, KugouLoginClient::Concept);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let error = provider
            .set_playlist_subscription(GID, true, if caller { None } else { Some("A") })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
        assert!(error.details.get("write_outcome").is_none());
        assert!(error.details.get("write_requests_dispatched").is_none());
        assert!(!error.retryable);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 4);
        assert!(
            all[2..]
                .iter()
                .all(|request| request.starts_with("POST /cloudlist.service/v8/get_all_list?"))
        );
        assert_eq!(body(&all[3])["page"], 2);
    }
}

#[tokio::test]
async fn concept_new_collection_provider_preserves_existing_noop_for_both_owners_and_references() {
    for caller in [false, true] {
        for id in [GID, COLLECTED] {
            // The matching membership appears only on the second directory page.
            let mut frames = start();
            frames.push(
                reply(json!({"userid":111,"total_ver":9,"list_count":31,
                "info":(1..=30).map(row).collect::<Vec<_>>()}))
                .into(),
            );
            frames.push(
                reply(json!({"userid":111,"total_ver":9,"list_count":31,
                "info":[collection(37)]}))
                .into(),
            );
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
                .set_playlist_subscription(id, true, if caller { None } else { Some("A") })
                .await
                .unwrap();
            assert!(result.subscribed);
            assert_eq!(result.extensions["changed"], false);
            assert_eq!(result.extensions["write_requests_dispatched"], 0);
            assert_eq!(
                result.extensions["account_playlist_ref"],
                format!("kugou:{COLLECTED}")
            );
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            let all = f.requests.await.unwrap();
            assert_eq!(all.len(), 4);
            assert!(
                all[2..]
                    .iter()
                    .all(|request| request.starts_with("POST /cloudlist.service/v8/get_all_list?"))
            );
            assert_eq!(body(&all[3])["page"], 2);
        }
    }
}

#[tokio::test]
async fn concept_single_uncollect_provider_binds_complete_library_and_both_ownerships() {
    for caller in [false, true] {
        for id in [GID, COLLECTED] {
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
            before.push(collection(37));
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
                .set_playlist_subscription(id, false, if caller { None } else { Some("A") })
                .await
                .unwrap();
            assert_eq!(result.resource_ref.id(), id);
            assert!(!result.subscribed);
            assert_eq!(
                result.extensions["account_playlist_ref"],
                format!("kugou:{COLLECTED}")
            );
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
                json!({"userid":111,"token":"next","listid":37,"total_ver":9,"type":1})
            );
            assert_eq!(
                all.iter()
                    .filter(|r| r.starts_with("POST /cloudlist.service/v3/delete_list?"))
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn concept_single_uncollect_provider_noops_absence_and_rejects_ambiguous_or_foreign_targets()
{
    for case in ["absent", "missing_gid", "duplicate_gid", "missing_local"] {
        let mut rows = vec![row(3)];
        if case == "missing_gid" {
            let mut item = collection(37);
            item.as_object_mut().unwrap().remove("list_create_gid");
            rows.push(item);
        } else if case == "duplicate_gid" {
            rows.extend([collection(37), collection(38)]);
        }
        let mut frames = start();
        frames.push(library(rows, 9).into());
        let mut f = server(frames).await;
        store_client(&mut f.provider, KugouLoginClient::Concept);
        let result = f
            .provider
            .set_playlist_subscription(
                if case == "missing_local" {
                    COLLECTED
                } else {
                    GID
                },
                false,
                Some("A"),
            )
            .await;
        if case == "absent" {
            let result = result.unwrap();
            assert!(!result.subscribed);
            assert_eq!(result.extensions["changed"], false);
            assert_eq!(result.extensions["write_requests_dispatched"], 0);
        } else {
            let e = result.unwrap_err();
            assert!(e.details.get("write_outcome").is_none(), "{case}");
        }
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let mut f = server(vec![]).await;
    store_client(&mut f.provider, KugouLoginClient::Concept);
    for id in ["cloudlist:222:1:37", REF] {
        assert!(
            f.provider
                .set_playlist_subscription(id, false, Some("A"))
                .await
                .is_err()
        );
    }
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_single_uncollect_provider_requires_exact_removal_and_matching_versions() {
    for case in [
        "retained",
        "replaced",
        "unrelated",
        "missing_other",
        "added",
        "version",
        "count",
        "uid",
    ] {
        let mut after = vec![row(3)];
        let mut ack = receipt(1);
        let mut uid = 111;
        match case {
            "retained" => after.push(collection(37)),
            "replaced" => after.push(collection(38)),
            "unrelated" => after[0]["name"] = json!("Changed elsewhere"),
            "missing_other" => after.clear(),
            "added" => after.push(row(38)),
            "version" => ack["total_ver"] = json!(11),
            "count" => ack["list_count"] = json!(2),
            _ => uid = 222,
        }
        let mut frames = initial();
        frames.push(reply(ack).into());
        frames.push(
            reply(json!({"userid":uid,"total_ver":10,"list_count":after.len(),"info":after}))
                .into(),
        );
        let mut f = server(frames).await;
        store_client(&mut f.provider, KugouLoginClient::Concept);
        let e = f
            .provider
            .set_playlist_subscription(GID, false, Some("A"))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict, "{case}");
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert_eq!(
            e.details["unconfirmed_refs"],
            json!([format!("kugou:{COLLECTED}")])
        );
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn concept_single_uncollect_provider_rejects_bad_ack_once_without_exposing_secrets() {
    for caller in [false, true] {
        for response in [
            json!({"status":0,"error_code":20010,"data":"private-response-marker"}),
            json!({"status":0,"error_code":20017}),
            json!({"status":1,"data":{"info":{"listid":37,"type":1,"code":1}}}),
            json!({"status":1,"data":{"userid":222,"total_ver":10,"pre_total_ver":9,"list_count":1}}),
            json!({"status":1,"data":{"userid":111,"total_ver":10,"pre_total_ver":8,"list_count":1}}),
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
                .set_playlist_subscription(GID, false, if caller { None } else { Some("A") })
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
async fn concept_single_uncollect_provider_relogin_discards_late_write_and_readback_results() {
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
                    p.set_playlist_subscription(GID, false, if caller { None } else { Some("A") })
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
