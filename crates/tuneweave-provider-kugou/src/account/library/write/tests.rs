use super::*;
use crate::account::tests::{frame, ok, request, server, session};

fn track() -> Track {
    let mut t = Track::new(ResourceRef::new(Platform::Kugou, "900").unwrap(), "Song");
    t.artists.push(ArtistSummary {
        resource_ref: None,
        name: "Artist".into(),
    });
    t.duration_ms = Some(216789);
    t.extensions.insert(
        "qualities".into(),
        json!({"standard":{"hash":"ABCDEF0123456789ABCDEF0123456789","size":120000}}),
    );
    t
}
fn ack() -> Value {
    json!({"userid":123456789,"listid":37,"type":0,"pre_list_ver":3,"list_ver":4,"count":2})
}

#[test]
fn write_inputs_use_verified_catalogue_identity_and_keep_unknown_wire_units_explicit() {
    let row = TrackInput::from_catalogue(&track(), "900").unwrap();
    let wire = serde_json::to_value(&row).unwrap();
    assert_eq!(
        wire,
        json!({"number":1,"name":"Artist - Song","hash":"abcdef0123456789abcdef0123456789",
        "size":120000,"sort":0,"timelen":0,"bitrate":0,"album_id":0,"mixsongid":900})
    );
    assert!(TrackInput::from_catalogue(&track(), "901").is_err());
    let mut t = track();
    t.extensions.get_mut("qualities").unwrap()["standard"]["hash"] = json!("short");
    assert!(TrackInput::from_catalogue(&t, "900").is_err());
    let mut t = track();
    t.name = "\0".into();
    assert!(TrackInput::from_catalogue(&t, "900").is_err());
}

#[tokio::test]
async fn native_track_writes_sign_exact_bodies_and_use_observed_routes_for_each_client() {
    for kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for add in [false, true] {
            if kind == KugouLoginClient::Concept && !add {
                // Ordinary Concept removal uses the separately tested versioned producer.
                continue;
            }
            let source = session(kind);
            let rows = [TrackInput::from_catalogue(&track(), "900").unwrap()];
            let (client, task) = server(vec![ok(ack())]).await;
            let result = client
                .native_write_tracks(
                    &source,
                    37,
                    if add {
                        Write::Add(&rows)
                    } else {
                        Write::Remove(&[81, 95])
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.version, Some(4));
            assert_eq!(result.previous_version, Some(3));
            assert_eq!(result.count, Some(2));
            let requests = task.await.unwrap();
            assert_eq!(requests.len(), 1);
            let (query, body) = request(
                &requests[0],
                if add && kind == KugouLoginClient::Concept {
                    Endpoint::LibraryAddConcept
                } else if add {
                    Endpoint::LibraryAdd
                } else {
                    Endpoint::LibraryRemove
                },
                kind,
            );
            assert_eq!(body["userid"], 123456789);
            assert_eq!(body["token"], source.token);
            assert_eq!(body["listid"], 37);
            assert_eq!(body["type"], 0);
            assert_eq!(body["list_ver"], 0);
            if add {
                assert_eq!(query["last_time"], query["clienttime"]);
                assert_eq!(query["last_area"], "gztx");
                assert_eq!(body["scene"], "false;null");
                assert_eq!(body["data"][0]["mixsongid"], 900);
            } else {
                assert!(!query.contains_key("last_time"));
                assert_eq!(body["data"], json!([{"fileid":81},{"fileid":95}]));
            }
        }
    }
}

#[tokio::test]
async fn native_write_acknowledgements_reject_wrong_account_list_type_and_explicit_rejections() {
    let source = session(KugouLoginClient::Standard);
    for (field, value, code) in [
        ("userid", json!(222), ErrorCode::Conflict),
        ("listid", json!(99), ErrorCode::Conflict),
        ("type", json!(1), ErrorCode::Conflict),
        ("code", json!(205), ErrorCode::UpstreamError),
    ] {
        let mut response = ack();
        response[field] = value;
        let (client, task) = server(vec![ok(response)]).await;
        assert_eq!(
            client
                .native_write_tracks(&source, 37, Write::Remove(&[81]))
                .await
                .err()
                .unwrap()
                .code,
            code
        );
        task.await.unwrap();
    }
    for code in [20010, 20017] {
        let (client, task) = server(vec![frame(
            200,
            "Content-Type: application/json\r\n",
            json!({"status":0,"error_code":code,"data":"unique-write-secret"})
                .to_string()
                .into_bytes(),
        )])
        .await;
        let e = client
            .native_write_tracks(&source, 37, Write::Remove(&[81]))
            .await
            .err()
            .unwrap();
        assert_eq!(
            e.code,
            if code == 20017 {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert!(!format!("{e:?}").contains("unique-write-secret"));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn native_writes_bound_json_and_do_not_retry_redirect_rate_or_auth_failures() {
    let source = session(KugouLoginClient::Standard);
    for (response, code) in [
        (
            frame(302, "Location: https://evil.invalid\r\n", vec![]),
            ErrorCode::UpstreamError,
        ),
        (
            frame(429, "Retry-After: 1\r\n", vec![]),
            ErrorCode::RateLimited,
        ),
        (frame(401, "", vec![]), ErrorCode::AuthenticationRequired),
        (
            frame(200, "Content-Type: text/html\r\n", b"{}".to_vec()),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                vec![b' '; 1_048_577],
            ),
            ErrorCode::UpstreamError,
        ),
    ] {
        let (client, task) = server(vec![response]).await;
        assert_eq!(
            client
                .native_write_tracks(&source, 37, Write::Remove(&[81]))
                .await
                .err()
                .unwrap()
                .code,
            code
        );
        assert_eq!(task.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn empty_duplicate_and_oversized_write_batches_fail_before_transport() {
    let source = session(KugouLoginClient::Standard);
    let (client, task) = server(vec![]).await;
    for ids in [vec![], vec![0], vec![1, 1], (1..=301).collect()] {
        assert!(
            client
                .native_write_tracks(&source, 37, Write::Remove(&ids))
                .await
                .is_err()
        );
    }
    assert!(
        client
            .native_write_tracks(&source, 37, Write::Add(&[]))
            .await
            .is_err()
    );
    assert!(task.await.unwrap().is_empty());
}
