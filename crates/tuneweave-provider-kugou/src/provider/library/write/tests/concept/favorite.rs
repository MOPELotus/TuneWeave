use super::*;

fn liked_list(version: u64) -> Value {
    let mut liked = list(version);
    liked["is_def"] = json!(2);
    liked["name"] = json!("我喜欢");
    liked
}
fn directory(liked: Value) -> Vec<Value> {
    let mut default = list_row(1, 0);
    default["is_def"] = json!(1);
    // A display name must not redirect writes into the default collection.
    default["name"] = json!("我喜欢");
    vec![default, liked]
}
fn library_frames(lists: &[Value]) -> Vec<Frame> {
    lists
        .chunks(30)
        .map(|chunk| {
            reply(json!({"userid":111,"total_ver":9,"list_count":lists.len(),"info":chunk})).into()
        })
        .collect()
}
fn favorite_snapshot(rows: Vec<Value>, version: u64, lists: &[Value]) -> Vec<Frame> {
    let mut frames = library_frames(lists);
    if rows.is_empty() {
        frames.push(
            reply(json!({"userid":111,"listid":37,"type":0,
            "list_ver":version,"count":0,"info":[]}))
            .into(),
        );
    } else {
        for chunk in rows.chunks(300) {
            frames.push(
                reply(json!({"userid":111,"listid":37,"type":0,
                "list_ver":version,"count":rows.len(),"info":chunk}))
                .into(),
            );
        }
    }
    frames.extend(library_frames(lists));
    frames
}

#[tokio::test]
async fn concept_favorite_remove_uses_unique_is_def_two_and_complete_pages_for_each_owner() {
    for caller in [false, true] {
        let lists = |version| {
            let mut lists = directory(liked_list(version));
            lists.extend((40..70).map(|id| list_row(id, 0)));
            lists
        };
        let after = (0..300)
            .map(|i| row(i + 100, i + 1000, i))
            .collect::<Vec<_>>();
        let mut before = after.clone();
        before.insert(150, row(777, 900, 150));
        // Keep an unambiguous source order when adding the selected occurrence.
        for (position, row) in before.iter_mut().enumerate() {
            row["sort"] = json!(position);
        }
        let mut frames = start();
        frames.extend(favorite_snapshot(before, 7, &lists(7)));
        frames.push(acknowledgement(7, 8, 300));
        frames.extend(favorite_snapshot(after, 8, &lists(8)));
        let count = frames.len();
        let mut f = server(frames).await;
        let store = concept_store(&mut f.provider);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let p = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let result = p
            .set_track_subscription("900", false, if caller { None } else { Some("A") })
            .await
            .unwrap();
        assert!(!result.subscribed);
        assert_eq!(result.resource_ref.id(), "900");
        assert_eq!(
            result.extensions["favorite_playlist_ref"],
            "kugou:cloudlist:111:0:37"
        );
        assert_eq!(result.extensions["affected_occurrences"], 1);
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(p.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), count);
        let writes = all
            .iter()
            .filter(|r| r.starts_with("POST /cloudlist.service/v4/delete_songs?"))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            body(writes[0]),
            json!({"userid":111,"token":"next","listid":37,
            "type":0,"list_ver":7,"data":[{"fileid":777}]})
        );
        assert_eq!(
            all.iter()
                .filter(|r| r.starts_with("POST /cloudlist.service/v8/") && body(r)["page"] == 2)
                .count(),
            4
        );
        assert!(all.iter().all(|r| !r.contains("modify_list")
            && !r.contains("upload")
            && !r.starts_with("POST /v4/delete_songs?")));
    }
}

#[tokio::test]
async fn concept_favorite_remove_rejects_missing_ambiguous_or_nonfavorite_identity_before_write() {
    for case in [
        "default_only",
        "missing_class",
        "wrong_kind",
        "two_favorites",
        "wrong_owner",
    ] {
        let mut liked = liked_list(7);
        match case {
            "default_only" => liked["is_def"] = json!(0),
            "missing_class" => {
                liked.as_object_mut().unwrap().remove("is_def");
            }
            "wrong_kind" => liked["type"] = json!(1),
            "wrong_owner" => liked["list_create_userid"] = json!(222),
            _ => {}
        }
        let mut lists = directory(liked);
        if case == "two_favorites" {
            let mut second = list_row(38, 0);
            second["is_def"] = json!(2);
            lists.push(second);
        }
        let mut frames = start();
        frames.extend(library_frames(&lists));
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let e = f
            .provider
            .set_track_subscription("900", false, Some("A"))
            .await
            .unwrap_err();
        assert!(e.details.get("write_outcome").is_none(), "{case}");
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn concept_favorite_remove_absence_is_noop_and_repeated_track_is_never_ambiguous_write() {
    for caller in [false, true] {
        for duplicate in [false, true] {
            let rows = if duplicate {
                vec![row(81, 900, 0), row(95, 900, 1)]
            } else {
                vec![row(70, 800, 0)]
            };
            let mut frames = start();
            frames.extend(favorite_snapshot(rows, 7, &directory(liked_list(7))));
            let mut f = server(frames).await;
            let store = concept_store(&mut f.provider);
            let saved = read(&store, "A");
            let p = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let result = p
                .set_track_subscription("900", false, if caller { None } else { Some("A") })
                .await;
            if duplicate {
                let e = result.unwrap_err();
                assert_eq!(e.code, ErrorCode::CapabilityNotSupported);
                assert!(e.details.get("write_outcome").is_none());
            } else {
                let result = result.unwrap();
                assert!(!result.subscribed);
                assert_eq!(result.extensions["changed"], false);
                assert_eq!(result.extensions["write_requests_dispatched"], 0);
            }
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            assert_eq!(f.requests.await.unwrap().len(), 5);
        }
    }
}

#[tokio::test]
async fn concept_favorite_remove_rejects_names_that_require_an_unproved_cover_followup() {
    for name in ["Renamed", " 我喜欢", "我喜欢 "] {
        let mut liked = liked_list(7);
        liked["name"] = json!(name);
        let mut frames = start();
        frames.extend(favorite_snapshot(
            vec![row(81, 900, 0)],
            7,
            &directory(liked),
        ));
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let e = f
            .provider
            .set_track_subscription("900", false, Some("A"))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::CapabilityNotSupported);
        assert!(e.details.get("write_outcome").is_none());
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn concept_favorite_remove_requires_same_favorite_exact_retained_order_and_ack() {
    for case in [
        "retained",
        "other_fileid",
        "other_order",
        "metadata",
        "replacement_favorite",
        "ack_count",
        "ack_version",
    ] {
        let before = vec![row(70, 800, 0), row(81, 900, 1), row(95, 801, 2)];
        let mut after = vec![row(70, 800, 0), row(95, 801, 2)];
        let mut liked = liked_list(8);
        let mut count = 2;
        let mut version = 8;
        match case {
            "retained" => after.insert(1, row(81, 900, 1)),
            "other_fileid" => after[0]["fileid"] = json!(71),
            "other_order" => after[0]["sort"] = json!(3),
            "metadata" => liked["name"] = json!("Unexpected rename"),
            "replacement_favorite" => liked["is_def"] = json!(0),
            "ack_count" => count = 3,
            _ => version = 9,
        }
        let mut frames = start();
        frames.extend(favorite_snapshot(before, 7, &directory(liked_list(7))));
        frames.push(acknowledgement(7, version, count));
        if case == "replacement_favorite" {
            // A new list taking over the favorite role must not confirm the old write.
            let mut replacement = liked_list(8);
            replacement["listid"] = json!(38);
            let mut lists = directory(liked);
            lists.push(replacement);
            frames.extend(library_frames(&lists));
            // The refreshed favorite's new list ID is verified again in the v3 response.
            frames.push(
                reply(json!({"userid":111,"listid":38,"type":0,
                "list_ver":8,"count":after.len(),"info":after}))
                .into(),
            );
            frames.extend(library_frames(&lists));
        } else {
            frames.extend(favorite_snapshot(after, 8, &directory(liked)));
        }
        let n = frames.len();
        let mut f = server(frames).await;
        concept_store(&mut f.provider);
        let e = f
            .provider
            .set_track_subscription("900", false, Some("A"))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed", "{case}");
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), n);
    }
}

#[tokio::test]
async fn concept_favorite_remove_rejects_late_write_or_readback_success_and_failure_after_relogin()
{
    for caller in [false, true] {
        for readback in [false, true] {
            for failed in [false, true] {
                let mut frames = start();
                frames.extend(favorite_snapshot(
                    vec![row(70, 800, 0), row(81, 900, 1)],
                    7,
                    &directory(liked_list(7)),
                ));
                if readback {
                    frames.push(acknowledgement(7, 8, 1));
                }
                let response = if failed {
                    raw(json!({"status":0,"error_code":20017}))
                } else if readback {
                    reply(
                        json!({"userid":111,"total_ver":9,"list_count":2,"info":directory(liked_list(8))}),
                    )
                } else {
                    reply(json!({"userid":111,"listid":37,"list_ver":8,"pre_list_ver":7,"count":1}))
                };
                let (frame, resume) = paused(response);
                frames.push(frame);
                let n = frames.len();
                let mut f = server(frames).await;
                let store = concept_store(&mut f.provider);
                let saved = read(&store, "A");
                let other = read(&store, "B");
                let p = if caller {
                    f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
                } else {
                    f.provider.clone()
                };
                let worker = p.clone();
                let task = tokio::spawn(async move {
                    worker
                        .set_track_subscription("900", false, if caller { None } else { Some("A") })
                        .await
                });
                for _ in 0..n {
                    f.seen.recv().await.unwrap();
                }
                let mut session = saved.native().session.clone();
                session.token = "new-login".into();
                let replacement = KugouCredential::verified(session).unwrap();
                if caller {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() =
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
                assert_eq!(f.requests.await.unwrap().len(), n);
            }
        }
    }
}
