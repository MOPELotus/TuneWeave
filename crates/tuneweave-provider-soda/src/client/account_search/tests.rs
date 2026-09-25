use super::*;
pub(crate) fn page(kind: SearchKind, start: u32, total: u32) -> serde_json::Value {
    let end = (start + 20).min(total);
    let name = match kind {
        SearchKind::Track => "track",
        SearchKind::Album => "album",
        SearchKind::Artist => "artist",
        SearchKind::Playlist => "playlist",
        _ => panic!("fixture kind"),
    };
    let rows=(start..end).map(|n| {
        let id=(1000+n).to_string();
        let data=match kind {
            SearchKind::Track=>json!({"id":id,"name":format!("Song {n}"),"duration":120000,"artists":[{"id":"456","name":"Artist"}],"album":{"id":"900","name":"Album"},"video_model":"discard-private-player"}),
            SearchKind::Album=>json!({"id":id,"name":format!("Album {n}"),"count_tracks":3}),
            SearchKind::Artist=>json!({"id":id,"name":format!("Artist {n}"),"count_tracks":3}),
            SearchKind::Playlist=>json!({"id":id,"title":format!("Playlist {n}"),"count_tracks":3}),
            _=>unreachable!(),
        };
        json!({"entity":{name:data}})
    }).collect::<Vec<_>>();
    json!({"status_info":{"now":1,"now_ts_ms":1000},"result_groups":[{"id":format!("{name}s"),"data":rows,"has_more":end<total,"next_cursor":end.to_string()}]})
}
#[test]
fn account_search_four_resource_mappings_reject_ambiguous_groups_and_cursor_gaps() {
    for kind in [
        SearchKind::Track,
        SearchKind::Album,
        SearchKind::Artist,
        SearchKind::Playlist,
    ] {
        let p = parse_page(&serde_json::to_vec(&page(kind, 0, 30)).unwrap(), kind, 0).unwrap();
        assert_eq!(p.items.len(), 20);
        assert_eq!(p.next_cursor, Some(20));
        assert!(p.has_more);
        let p = parse_page(&serde_json::to_vec(&page(kind, 20, 30)).unwrap(), kind, 20).unwrap();
        assert_eq!(p.items.len(), 10);
        assert!(!p.has_more);
        for case in 0..9 {
            let mut v = page(kind, 0, 30);
            match case {
                0 => v["status_info"]["now"] = json!(0),
                1 => v["result_groups"][0]["next_cursor"] = json!("0"),
                2 => {
                    let duplicate = v["result_groups"][0]["data"][0].clone();
                    v["result_groups"][0]["data"]
                        .as_array_mut()
                        .unwrap()
                        .push(duplicate);
                }
                3 => {
                    let duplicate = v["result_groups"][0].clone();
                    v["result_groups"].as_array_mut().unwrap().push(duplicate);
                }
                4 => {
                    v["result_groups"][0]["data"][0]["entity"] =
                        json!({"track":{"id":"7","name":"Song"},"album":{"id":"7","name":"Album"}})
                }
                5 => v["result_groups"][0]["data"][19]["entity"] = json!({}),
                6 => v["result_groups"][0]["has_more"] = serde_json::Value::Null,
                7 => v["status_info"]["status_code"] = json!(77),
                _ => v["status_code"] = json!("0"),
            }
            assert!(
                parse_page(&serde_json::to_vec(&v).unwrap(), kind, 0).is_err(),
                "{kind:?}/{case}"
            );
        }
        let empty = json!({"status_info":{"now":1,"now_ts_ms":1000},"result_groups":[],"extra":{"empty_search":1}});
        assert!(
            parse_page(&serde_json::to_vec(&empty).unwrap(), kind, 0)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(parse_page(b"", kind, 0).is_err());
    }
}
#[tokio::test]
async fn account_search_sdk_fixed_pc_transport_cookie_rotation_and_installation_identity() {
    let kinds = [
        SearchKind::Track,
        SearchKind::Album,
        SearchKind::Artist,
        SearchKind::Playlist,
    ];
    let responses = kinds
        .iter()
        .map(|k| {
            crate::test_http::json(
                &page(*k, 0, 1).to_string(),
                Some("sessionid_ss=rotated-session; Path=/"),
            )
        })
        .collect();
    let (origin, server) = crate::test_http::serve(responses).await;
    let client = SodaClient::test_client().with_auth_test_origin(origin.clone());
    let c = SodaCredential::test_credential("selected-session")
        .bind_user("123456")
        .unwrap();
    let id = search_id();
    let mut budget = 4 * MAX_PAGE;
    for kind in kinds {
        let (mut p, next) = client
            .account_search_page(kind, " 周 & 杰 ", 0, &id, &c, &mut budget)
            .await
            .unwrap();
        assert!(c.same_login(&next));
        assert!(next.cookie_header().unwrap().contains("rotated-session"));
        let ext = item_extensions(&mut p.items[0]);
        assert_eq!(ext["source_user_id"], "123456");
        assert_eq!(ext["backend"], BACKEND);
    }
    let wires = server.await.unwrap();
    let mut devices = Vec::new();
    for (wire, kind) in wires.iter().zip(kinds) {
        assert!(wire.starts_with(&format!("GET {}?", path(kind).unwrap())));
        assert!(wire.contains("cookie: sessionid_ss=selected-session"));
        let u = origin
            .join(
                wire.lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap(),
            )
            .unwrap();
        let q = u.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(q["q"], "周 & 杰");
        assert_eq!(q["cursor"], "0");
        assert_eq!(q["search_id"], id);
        assert_eq!(q["search_method"], "input");
        assert_eq!(q["fp"], q["device_id"]);
        assert_ne!(q["device_id"], q["iid"]);
        assert_eq!(q["app_name"], "luna_pc");
        assert!(!q.contains_key("count"));
        devices.push((q["device_id"].to_string(), q["iid"].to_string()));
    }
    assert!(devices.windows(2).all(|v| v[0] == v[1]));
}
#[tokio::test]
async fn account_search_sdk_failure_and_body_budgets_never_accept_cookie_updates() {
    let c = SodaCredential::test_credential("selected-session")
        .bind_user("123456")
        .unwrap();
    for response in [
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into(),
        crate::test_http::json(r#"{"status_code":1000016}"#, None),
        crate::test_http::json(r#"{"status_info":{"status_code":1000016}}"#, None),
        crate::test_http::json("", None),
        crate::test_http::json(&page(SearchKind::Track, 0, 1).to_string(), None).replace(
            "Content-Type:",
            "bdturing-verify: challenge-fixture\r\nContent-Type:",
        ),
        crate::test_http::json(&page(SearchKind::Track, 0, 1).to_string(), None)
            .replace("application/json", "text/html"),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            MAX_PAGE + 1
        ),
        crate::test_http::json(
            &page(SearchKind::Track, 0, 1).to_string(),
            Some("sessionid_ss=; Max-Age=0; Path=/"),
        ),
    ] {
        let (origin, server) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let mut budget = MAX_PAGE;
        assert!(
            client
                .account_search_page(SearchKind::Track, "q", 0, &search_id(), &c, &mut budget)
                .await
                .is_err()
        );
        server.await.unwrap();
    }
    let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
        &page(SearchKind::Track, 0, 1).to_string(),
        None,
    )])
    .await;
    let client = SodaClient::test_client().with_auth_test_origin(origin);
    assert!(
        client
            .account_search_page(SearchKind::Track, "q", 0, &search_id(), &c, &mut 8)
            .await
            .is_err()
    );
    server.await.unwrap();
}
#[test]
fn account_search_rejects_raw_encoded_and_later_rotated_session_values() {
    let sources = [
        SodaCredential::test_credential("first-secret"),
        SodaCredential::test_credential("later-secret"),
    ];
    for value in [
        json!({"title":"first-secret"}),
        json!({"cover":"https://p3-luna.douyinpic.com/%6cater-secret"}),
        json!({"nested":[{"later-secret":"value"}]}),
    ] {
        assert!(reject_secrets(&value, &sources).is_err());
    }
    assert!(reject_secrets(&json!({"title":"An ordinary song"}), &sources).is_ok());
}

#[tokio::test]
#[ignore = "official anonymous PC catalogue metadata only, no account or media"]
async fn live_soda_pc_search_catalogue_pages_validate_actual_groups_and_terminal_album_page() {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    for kind in [SearchKind::Album, SearchKind::Artist, SearchKind::Playlist] {
        let id = search_id();
        for cursor in [0, 20] {
            let response = client
                .get(format!("https://api.qishui.com{}", path(kind).unwrap()))
                .query(&[
                    ("aid", "386088"),
                    ("app_name", "luna_pc"),
                    ("device_platform", "windows"),
                    ("version_name", "2.1.0"),
                    ("version_code", "20010000"),
                    ("channel", "official"),
                    ("q", "周杰伦"),
                    ("cursor", &cursor.to_string()),
                    ("search_id", &id),
                    ("search_method", "input"),
                    ("debug_params", ""),
                    ("from_search_id", ""),
                    ("search_scene", ""),
                ])
                .send()
                .await
                .unwrap();
            let body = read_bounded_response(response, "Soda public PC search test")
                .await
                .unwrap();
            let p = parse_page(&body, kind, cursor).unwrap();
            assert!(!p.items.is_empty());
            if p.has_more {
                assert_eq!(p.next_cursor, Some(cursor + 20));
            }
        }
    }
}
