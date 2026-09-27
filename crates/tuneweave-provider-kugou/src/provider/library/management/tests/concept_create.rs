use super::*;

fn request(account: Option<&str>) -> PlaylistCreateRequest {
    PlaylistCreateRequest {
        name: "Created".into(),
        visibility: PlaylistVisibility::PlatformDefault,
        kind: PlaylistKind::Normal,
        account: account.map(str::to_owned),
    }
}
fn receipt() -> Value {
    json!({"userid":111,"total_ver":10,"pre_total_ver":9,"list_count":2,
        "info":{"code":1,"listid":37,"type":0,"source":1,"name":"Created"}})
}
fn initial() -> Vec<Frame> {
    let mut frames = start();
    frames.push(library(vec![row(3)], 9).into());
    frames
}
fn created() -> Value {
    let mut r = row(37);
    r["name"] = json!("Created");
    r.as_object_mut().unwrap().remove("is_def");
    r
}

#[tokio::test]
async fn concept_default_create_provider_preserves_observed_visibility_without_a_guarantee() {
    for caller in [false, true] {
        for private in [Some(0), Some(1), None] {
            let mut new = created();
            if let Some(v) = private {
                new["is_pri"] = json!(v);
            } else {
                new.as_object_mut().unwrap().remove("is_pri");
            }
            let mut frames = initial();
            frames.push(reply(receipt()).into());
            frames.push(library(vec![row(3), new], 10).into());
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
                .create_playlist(&request(if caller { None } else { Some("A") }))
                .await
                .unwrap();
            assert_eq!(result.playlist_ref.id(), REF);
            assert_eq!(
                result.extensions["requested_visibility"],
                "platform_default"
            );
            assert_eq!(result.extensions["visibility_guaranteed"], false);
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            assert_eq!(
                result
                    .playlist
                    .unwrap()
                    .extensions
                    .get("is_private")
                    .and_then(Value::as_bool),
                private.map(|n| n == 1)
            );
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            let all = f.requests.await.unwrap();
            assert_eq!(all.len(), 5);
            assert!(all[3].starts_with("POST /cloudlist.service/v4/add_list?"));
            assert!(body(&all[3]).get("is_pri").is_none());
            assert_eq!(body(&all[3])["total_ver"], 9);
        }
    }
}

#[tokio::test]
async fn concept_default_create_provider_rejects_unsupported_visibility_and_long_names_before_io() {
    for (client, visibility) in [
        (KugouLoginClient::Concept, PlaylistVisibility::Public),
        (KugouLoginClient::Concept, PlaylistVisibility::Private),
        (
            KugouLoginClient::Standard,
            PlaylistVisibility::PlatformDefault,
        ),
    ] {
        let mut f = server(vec![]).await;
        store_client(&mut f.provider, client);
        let mut r = request(Some("A"));
        r.visibility = visibility;
        assert_eq!(
            f.provider.create_playlist(&r).await.unwrap_err().code,
            ErrorCode::CapabilityNotSupported
        );
        assert!(f.requests.await.unwrap().is_empty());
    }
    let mut f = server(vec![]).await;
    store_client(&mut f.provider, KugouLoginClient::Concept);
    let mut r = request(Some("A"));
    r.name = "界".repeat(21);
    assert_eq!(
        f.provider.create_playlist(&r).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_default_create_provider_requires_exact_new_list_and_unchanged_other_lists() {
    for case in [
        "name",
        "nonempty",
        "unrelated",
        "version",
        "gid",
        "system_marker",
    ] {
        let mut new = created();
        let mut old = row(3);
        let mut ack = receipt();
        match case {
            "name" => new["name"] = json!("Wrong"),
            "nonempty" => new["count"] = json!(1),
            "system_marker" => new["is_def"] = json!(2),
            "unrelated" => old["name"] = json!("Changed"),
            "version" => ack["total_ver"] = json!(11),
            _ => ack["info"]["global_collection_id"] = json!("collection_1_111_99_0"),
        }
        let mut frames = initial();
        frames.push(reply(ack).into());
        frames.push(library(vec![old, new], 10).into());
        let mut f = server(frames).await;
        store_client(&mut f.provider, KugouLoginClient::Concept);
        let e = f
            .provider
            .create_playlist(&request(Some("A")))
            .await
            .unwrap_err();
        assert_eq!(
            e.code,
            if case == "system_marker" {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::Conflict
            },
            "{case}"
        );
        assert_eq!(e.details["write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
    let mut frames = initial();
    let mut ack = receipt();
    ack["info"]["listid"] = json!(3);
    frames.push(reply(ack).into());
    let mut f = server(frames).await;
    store_client(&mut f.provider, KugouLoginClient::Concept);
    assert_eq!(
        f.provider
            .create_playlist(&request(Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(f.requests.await.unwrap().len(), 4);
}

#[tokio::test]
async fn concept_default_create_provider_reads_every_before_and_after_page() {
    let snapshot = |rows: Vec<Value>, version: u64| {
        rows.chunks(30)
            .map(|chunk| {
                Frame::from(reply(json!({"userid":111,
        "total_ver":version,"list_count":rows.len(),"info":chunk})))
            })
            .collect::<Vec<_>>()
    };
    let before = (1..=31).map(row).collect::<Vec<_>>();
    let mut after = before.clone();
    let mut new = row(100);
    new["name"] = json!("Created");
    after.push(new);
    let mut ack = receipt();
    ack["list_count"] = json!(32);
    ack["info"]["listid"] = json!(100);
    let mut frames = start();
    frames.extend(snapshot(before, 9));
    frames.push(reply(ack).into());
    frames.extend(snapshot(after, 10));
    let mut f = server(frames).await;
    store_client(&mut f.provider, KugouLoginClient::Concept);
    let result = f
        .provider
        .create_playlist(&request(Some("A")))
        .await
        .unwrap();
    assert_eq!(result.playlist_ref.id(), "cloudlist:111:0:100");
    let all = f.requests.await.unwrap();
    assert_eq!(all.len(), 7);
    assert_eq!(body(&all[3])["page"], 2);
    assert_eq!(body(&all[6])["page"], 2);
}

#[tokio::test]
async fn concept_default_create_provider_relogin_discards_late_success_and_error() {
    for caller in [false, true] {
        for failed in [false, true] {
            let (frame, resume) = paused(if failed {
                raw(json!({"status":0,"error_code":20017}))
            } else {
                reply(receipt())
            });
            let mut frames = initial();
            frames.push(frame);
            let mut f = server(frames).await;
            let store = store_client(&mut f.provider, KugouLoginClient::Concept);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let p = provider.clone();
            let task = tokio::spawn(async move {
                p.create_playlist(&request(if caller { None } else { Some("A") }))
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
            let mut e = task.await.unwrap().unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict);
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(e.take_caller_credential_update().is_none());
            assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}
