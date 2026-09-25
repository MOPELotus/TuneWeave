use super::*;

fn profile(tracks: Option<u64>, albums: Option<u64>) -> serde_json::Value {
    let mut value = json!({"status_info":{"now":1,"now_ts_ms":1000},"artist_info":{"id":"123","name":"Artist"}});
    if let Some(n) = tracks {
        value["artist_info"]["count_tracks"] = json!(n);
    }
    if let Some(n) = albums {
        value["artist_info"]["count_albums"] = json!(n);
    }
    value
}

fn item(id: u64) -> serde_json::Value {
    json!({"id":id.to_string(),"name":format!("Title {id}"),"duration":1000,
        "artists":[{"id":"789","name":"Collaborator"},{"id":"123","name":"Artist"}]})
}

fn page(kind: &str, ids: &[u64], more: Option<bool>, cursor: &str) -> serde_json::Value {
    let mut value = json!({"status_info":{"now":1,"now_ts_ms":1000},"next_cursor":cursor});
    value[kind] = json!(ids.iter().map(|id| item(*id)).collect::<Vec<_>>());
    if let Some(more) = more {
        value["has_more"] = json!(more);
    }
    value
}

async fn client(
    responses: Vec<serde_json::Value>,
) -> (SodaClient, tokio::task::JoinHandle<Vec<String>>) {
    let (origin, server) = crate::test_http::serve(
        responses
            .into_iter()
            .map(|j| {
                crate::test_http::json(&j.to_string(), Some("sessionid_ss=not-an-account; Path=/"))
            })
            .collect(),
    )
    .await;
    (
        SodaClient::new(&SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin),
        server,
    )
}

#[tokio::test]
async fn artist_catalogue_follows_scan_cursor_and_preserves_complete_collaborations() {
    let ids = (1..=50).collect::<Vec<_>>();
    let (client, server) = client(vec![
        profile(Some(52), None),
        page("tracks", &ids, Some(true), "99"),
        page("tracks", &[51, 52], None, "401"),
    ])
    .await;
    let result = client.artist_catalog_tracks("123").await.unwrap();
    assert_eq!(result.items.len(), 52);
    assert_eq!(result.reported_total, Some(52));
    assert_eq!(result.upstream_pages, 2);
    assert_eq!(result.items.last().unwrap().id, "52");
    assert_eq!(result.items[0].artists.len(), 2);
    assert_eq!(result.items[0].artists[0].name, "Collaborator");
    assert_eq!(
        result.items[0].extensions["backend"],
        "official_pc_artist_tracks"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    for (i, request) in requests.iter().enumerate() {
        assert!(!request.to_ascii_lowercase().contains("cookie:"));
        assert!(!request.contains("device_id="));
        assert!(!request.contains("iid="));
        let target = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = Url::parse(&format!("https://api.qishui.com{target}")).unwrap();
        let q = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(q["app_name"], "luna_pc");
        assert_eq!(q["version_code"], "20010000");
        if i == 0 {
            assert_eq!(url.path(), "/luna/pc/artists/123");
            assert!(!q.contains_key("cursor"));
        } else {
            assert_eq!(url.path(), "/luna/pc/artists/123/tracks");
            assert_eq!(q["count"], "50");
            assert_eq!(q["cursor"], if i == 1 { "" } else { "99" });
        }
    }
}

#[tokio::test]
async fn artist_albums_keep_unknown_track_counts_and_omitted_empty_arrays() {
    let mut albums = page("albums", &[456, 457], None, "88");
    albums["albums"][1]["count_tracks"] = json!(9);
    let (client, server) = client(vec![profile(None, Some(2)), albums]).await;
    let result = client.artist_catalog_albums("123").await.unwrap();
    assert_eq!(result.items[0].track_count, None);
    assert_eq!(result.items[1].track_count, Some(9));
    assert_eq!(result.items[0].artists.len(), 2);
    assert_eq!(
        result.items[0].extensions["backend"],
        "official_pc_artist_albums"
    );
    assert_eq!(server.await.unwrap().len(), 2);
    let (client, server) = self::client(vec![
        profile(None, None),
        json!({"status_info":{"now":1,"now_ts_ms":1000},"next_cursor":"82"}),
    ])
    .await;
    let result = client.artist_catalog_albums("123").await.unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.reported_total, None);
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn artist_catalogue_rejects_late_duplicates_wrong_credits_and_count_drift() {
    let mut wrong = page("tracks", &[2], None, "100");
    wrong["tracks"][0]["artists"] = json!([{"id":"789","name":"Other"}]);
    let mut bad_credit = page("tracks", &[2], None, "100");
    bad_credit["tracks"][0]["artists"][0]["id"] = json!("broken");
    let mut bad_status = page("tracks", &[2], None, "100");
    bad_status["status_code"] = json!(9);
    for last in [
        page("tracks", &[1], None, "100"),
        wrong,
        bad_credit,
        bad_status,
        page("tracks", &[], None, "100"),
        page("tracks", &[2, 3], None, "100"),
        page("albums", &[2], None, "100"),
    ] {
        let (client, server) = client(vec![
            profile(Some(2), None),
            page("tracks", &[1], Some(true), "50"),
            last,
        ])
        .await;
        assert_eq!(
            client
                .artist_catalog_tracks("123")
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(server.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn artist_catalogue_rejects_stalled_cursors_empty_continuations_and_false_termination() {
    for first in [
        page("tracks", &[1], Some(true), ""),
        page("tracks", &[1], Some(true), "0"),
        page("tracks", &[], Some(true), "50"),
        page("tracks", &[1], None, "99"),
        page("tracks", &[1], Some(true), "01"),
        page("tracks", &[1], Some(true), "18446744073709551616"),
    ] {
        let (client, server) = client(vec![profile(Some(2), None), first]).await;
        assert!(client.artist_catalog_tracks("123").await.is_err());
        assert_eq!(server.await.unwrap().len(), 2);
    }
    let (client, server) = client(vec![
        profile(None, None),
        page("tracks", &[1], Some(true), "50"),
        page("tracks", &[2], Some(true), "50"),
    ])
    .await;
    assert!(client.artist_catalog_tracks("123").await.is_err());
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn artist_catalogue_requires_identity_and_rejects_explicit_null_or_invalid_status() {
    let (missing_client, missing_server) = client(vec![
        profile(None, None),
        json!({"status_info":{"now":1,"now_ts_ms":1000}}),
    ])
    .await;
    assert!(
        missing_client
            .artist_catalog_albums("123")
            .await
            .err()
            .unwrap()
            .message
            .contains("directory cursor")
    );
    assert_eq!(missing_server.await.unwrap().len(), 2);
    let mut wrong = profile(Some(1), None);
    wrong["artist_info"]["id"] = json!("789");
    for profile in [
        json!({}),
        json!({"status_info":{"now":1,"now_ts_ms":1000}}),
        wrong,
        profile(Some(10001), None),
    ] {
        let (client, server) = client(vec![profile]).await;
        assert!(client.artist_catalog_tracks("123").await.is_err());
        assert_eq!(server.await.unwrap().len(), 1);
    }
    for (key, value) in [
        ("tracks", json!(null)),
        ("has_more", json!(null)),
        ("next_cursor", json!(null)),
        ("status_info", json!({"now":1,"now_ts_ms":2000})),
        ("status_info", json!({"now":0,"now_ts_ms":0})),
    ] {
        let mut last = page("tracks", &[1], None, "99");
        last[key] = value;
        let (client, server) = client(vec![profile(Some(1), None), last]).await;
        assert!(client.artist_catalog_tracks("123").await.is_err());
        assert_eq!(server.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn artist_catalogue_enforces_page_and_item_budgets_without_partial_delivery() {
    let mut responses = vec![profile(None, None)];
    responses.extend((1..=128).map(|i| page("tracks", &[i], Some(true), &i.to_string())));
    let (client, server) = client(responses).await;
    let error = client.artist_catalog_tracks("123").await.err().unwrap();
    assert!(error.message.contains("page budget"));
    assert_eq!(server.await.unwrap().len(), 129);
    let (client, server) = self::client(vec![
        profile(None, None),
        page("tracks", &(1..=1001).collect::<Vec<_>>(), None, "2000"),
    ])
    .await;
    assert!(client.artist_catalog_tracks("123").await.is_err());
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn artist_catalogue_never_follows_redirects_or_accepts_html_and_large_responses() {
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: https://example.invalid/\r\nContent-Length: 0\r\n\r\n"
            .to_owned(),
        crate::test_http::json(&profile(Some(0), None).to_string(), None)
            .replace("application/json", "text/html"),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            MAX_API_RESPONSE_BYTES + 1
        ),
    ] {
        let (origin, server) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::new(&SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin);
        assert!(client.artist_catalog_tracks("123").await.is_err());
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn artist_catalogue_remaining_byte_budget_is_enforced_before_parsing_or_next_request() {
    let body = profile(Some(0), None).to_string();
    for response in [
        crate::test_http::json(&body, None),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}"
        ),
    ] {
        let (origin, server) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::new(&SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin);
        let error = client
            .artist_catalog_request("123", None, "", body.len() - 1, None)
            .await
            .unwrap_err();
        assert!(error.message.contains("size limit"));
        assert_eq!(server.await.unwrap().len(), 1);
        assert!(
            client
                .artist_catalog_request("123", None, "", 0, None)
                .await
                .unwrap_err()
                .message
                .contains("aggregate size limit")
        );
    }
}

#[tokio::test]
#[ignore = "requires official anonymous Soda artist metadata; no account or media is used"]
async fn live_artist_catalogue_reads_complete_tracks_albums_and_empty_albums() {
    let client = SodaClient::new(&SodaConfig::default()).unwrap();
    let tracks = client
        .artist_catalog_tracks("6754918579642042369")
        .await
        .unwrap();
    assert!(tracks.items.len() > 100);
    assert_eq!(tracks.reported_total, Some(tracks.items.len() as u64));
    assert!(tracks.upstream_pages > 1);
    let albums = client
        .artist_catalog_albums("6754918579642042369")
        .await
        .unwrap();
    assert!(!albums.items.is_empty());
    assert_eq!(albums.reported_total, Some(albums.items.len() as u64));
    // The public Jay catalogue may change; accept its verified complete response.
    let jay = client
        .artist_catalog_albums("6681166129722824706")
        .await
        .unwrap();
    assert!(
        jay.reported_total
            .is_none_or(|n| n == jay.items.len() as u64)
    );
}
