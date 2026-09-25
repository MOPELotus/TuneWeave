use super::*;

fn parse_fixture(v: &serde_json::Value) -> Result<ArtistOverview> {
    parse(&serde_json::to_vec(v).unwrap(), "123")
}

#[test]
fn pc_artist_detail_maps_profile_and_explicit_hot_preview_without_linked_user_secrets() {
    let p = parse_fixture(&fixture()).unwrap();
    assert_eq!(p.artist.id, "123");
    assert_eq!(p.artist.track_count, Some(3));
    assert_eq!(p.artist.album_count, Some(7));
    assert_eq!(p.artist.aliases, ["Alias"]);
    assert_eq!(p.artist.description, "A biography");
    assert_eq!(p.artist.extensions["linked_user_id"], "456");
    assert_eq!(p.artist.extensions["account_state"]["is_collected"], true);
    assert_eq!(p.artist.extensions["account_state"]["blocked_by_me"], false);
    assert!(p.has_more_tracks);
    assert_eq!(p.featured_tracks.len(), 2);
    assert_eq!(p.featured_tracks[0].artists.len(), 2);
    assert_eq!(p.extensions["preview_scope"], "hot_tracks");
    let text = serde_json::to_string(&p).unwrap();
    for secret in ["ignored-linked-user-secret", "ignored-hot-album-secret"] {
        assert!(!text.contains(secret));
    }
}

#[test]
fn pc_artist_detail_omitted_false_empty_preview_and_unknown_counts_remain_distinct() {
    let mut v = fixture();
    v["artist_info"]["count_tracks"] = json!(2);
    v.as_object_mut().unwrap().remove("has_more_tracks");
    assert!(!parse_fixture(&v).unwrap().has_more_tracks);
    v["artist_info"]["count_tracks"] = json!(0);
    v.as_object_mut().unwrap().remove("hot_tracks");
    let p = parse_fixture(&v).unwrap();
    assert!(p.featured_tracks.is_empty());
    assert!(!p.has_more_tracks);
    assert_eq!(p.artist.track_count, Some(0));
    for key in ["count_tracks", "count_albums", "state", "user"] {
        v["artist_info"].as_object_mut().unwrap().remove(key);
    }
    let p = parse_fixture(&v).unwrap();
    assert_eq!(p.artist.track_count, None);
    assert_eq!(p.artist.album_count, None);
    assert!(!p.artist.extensions.contains_key("account_state"));
    assert!(!p.artist.extensions.contains_key("linked_user_id"));
    v["hot_tracks"] = json!([]);
    v["has_more_tracks"] = json!(true);
    assert!(parse_fixture(&v).unwrap().has_more_tracks);
}

#[test]
fn pc_artist_detail_rejects_inconsistent_counts_identity_links_and_all_bad_preview_rows() {
    for case in [
        "profile",
        "identity",
        "too_few",
        "false_more",
        "false_omitted",
        "true_equal",
        "duplicate",
        "credit",
        "link_id",
        "link_artist",
        "album_count",
        "too_many",
        "null_tracks",
        "null_more",
        "bad_state",
        "bad_status",
    ] {
        let mut v = fixture();
        match case {
            "profile" => {
                v.as_object_mut().unwrap().remove("artist_info");
            }
            "identity" => v["artist_info"]["id"] = json!("456"),
            "too_few" => v["artist_info"]["count_tracks"] = json!(1),
            "false_more" => v["has_more_tracks"] = json!(false),
            "false_omitted" => {
                v.as_object_mut().unwrap().remove("has_more_tracks");
            }
            "true_equal" => v["artist_info"]["count_tracks"] = json!(2),
            "duplicate" => v["hot_tracks"][1]["id"] = json!("11"),
            "credit" => v["hot_tracks"][1]["artists"][0]["id"] = json!("789"),
            "link_id" => v["artist_info"]["user"]["id"] = json!("0456"),
            "link_artist" => v["artist_info"]["user"]["artist_id"] = json!("456"),
            "album_count" => v["artist_info"]["count_albums"] = json!(1_000_001),
            "too_many" => {
                v["artist_info"]["count_tracks"] = json!(101);
                v["has_more_tracks"] = json!(false);
                let first = v["hot_tracks"][0].clone();
                v["hot_tracks"] = json!(
                    (1..=101)
                        .map(|id| {
                            let mut t = first.clone();
                            t["id"] = json!(id.to_string());
                            t
                        })
                        .collect::<Vec<_>>()
                );
            }
            "null_tracks" => v["hot_tracks"] = json!(null),
            "null_more" => v["has_more_tracks"] = json!(null),
            "bad_state" => v["artist_info"]["state"]["is_collected"] = json!("true"),
            _ => v["status_info"]["now_ts_ms"] = json!(5000),
        }
        assert!(parse_fixture(&v).is_err(), "{case}");
    }
}

#[tokio::test]
async fn pc_artist_detail_sdk_sends_only_selected_cookie_and_accepts_rotation_after_full_validation()
 {
    for account in [false, true] {
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            &fixture().to_string(),
            Some("sessionid_ss=detail-new-session; Path=/"),
        )])
        .await;
        let client = SodaClient::new(&SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin);
        let source = SodaCredential::test_credential("detail-input-session")
            .bind_user("123456")
            .unwrap();
        let p = client
            .pc_artist_detail("123", account.then_some(&source))
            .await
            .unwrap();
        assert_eq!(p.credential.is_some(), account);
        assert_eq!(
            p.overview.artist.extensions.contains_key("source_user_id"),
            account
        );
        if account {
            assert_eq!(p.overview.artist.extensions["source_user_id"], "123456");
            assert_eq!(p.overview.artist.extensions["linked_user_id"], "456");
            assert!(
                p.credential
                    .unwrap()
                    .cookie_header()
                    .unwrap()
                    .contains("detail-new-session")
            );
        }
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 1);
        let wire = &seen[0];
        assert!(wire.starts_with("GET /luna/pc/artists/123?"));
        assert_eq!(
            wire.contains("cookie: sessionid_ss=detail-input-session"),
            account
        );
        assert_eq!(wire.contains("device_id="), account);
        assert_eq!(wire.contains("iid="), account);
        assert_eq!(wire.contains("fp="), account);
        assert!(!wire.contains("user_id="));
    }
}

#[tokio::test]
async fn pc_artist_detail_sdk_refuses_current_or_candidate_credential_reflection_before_returning_rotation()
 {
    for secret in ["detail-input-session", "detail-new-session"] {
        for place in ["name", "bio", "preview"] {
            let mut v = fixture();
            match place {
                "name" => v["artist_info"]["name"] = json!(secret),
                "bio" => v["artist_info"]["artist_profile"]["intro"] = json!(secret),
                _ => v["hot_tracks"][1]["name"] = json!(secret),
            }
            let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
                &v.to_string(),
                Some("sessionid_ss=detail-new-session; Path=/"),
            )])
            .await;
            let client = SodaClient::new(&SodaConfig::default())
                .unwrap()
                .with_auth_test_origin(origin);
            let source = SodaCredential::test_credential("detail-input-session")
                .bind_user("123456")
                .unwrap();
            let error = client
                .pc_artist_detail("123", Some(&source))
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(!format!("{error:?}").contains(secret));
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
#[ignore = "requires official anonymous artist metadata; no account or platform media"]
async fn live_pc_artist_detail_preserves_full_and_partial_hot_previews_and_separate_linked_users() {
    let client = SodaClient::new(&SodaConfig::default()).unwrap();
    for id in [
        "7112274641380444162",
        "6681166129722824706",
        "6754918579642042369",
    ] {
        let p = client.pc_artist_detail(id, None).await.unwrap();
        assert!(p.credential.is_none());
        let o = p.overview;
        assert_eq!(o.artist.id, id);
        assert!(!o.featured_tracks.is_empty());
        assert_eq!(
            o.has_more_tracks,
            o.artist.track_count.unwrap() > o.featured_tracks.len() as u64
        );
        for t in &o.featured_tracks {
            assert!(
                t.artists
                    .iter()
                    .any(|a| a.resource_ref.as_ref().is_some_and(|r| r.id() == id))
            );
        }
        if id == "7112274641380444162" {
            assert_eq!(o.featured_tracks.len(), 1);
            assert!(!o.has_more_tracks);
        } else {
            assert_eq!(o.featured_tracks.len(), 5);
            assert!(o.has_more_tracks);
            assert!(o.artist.extensions.contains_key("linked_user_id"));
        }
        let independent = client.artist_catalog_tracks(id).await.unwrap();
        for t in o.featured_tracks {
            assert!(
                independent
                    .items
                    .iter()
                    .any(|x| x.resource_ref == t.resource_ref)
            );
        }
    }
}
