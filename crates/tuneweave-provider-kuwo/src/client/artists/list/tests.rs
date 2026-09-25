use super::*;
use crate::client::catalog::tests::{json_response, requests, response, setup};
use std::collections::BTreeMap;
use tuneweave_core::{Capability, MusicProvider};

fn singer(id: u64) -> serde_json::Value {
    json!({
        "id": id, "name":format!("Singer {id} &amp; Friends"), "aartist":"Alias",
        "pic":"https://img1.kuwo.cn/star/starheads/300/s1/artist.jpg",
        "musicNum":23, "albumNum":"5", "mvNum":3, "artistFans":0,
        "isStar":0, "content_type":"0"
    })
}

fn body(page: u32, total: u64) -> serde_json::Value {
    let start = u64::from(page - 1) * 60;
    let items = (start..total.min(start + 60))
        .map(|id| singer(id + 1))
        .collect::<Vec<_>>();
    json!({"code":200,"data":{"total":total.to_string(),"artistList":items}})
}

fn wire(wire: &str, category: u8, prefix: &str, page: u32) -> String {
    let url = Url::parse(&format!(
        "https://wapi.kuwo.cn{}",
        wire.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(url.path(), PATH);
    let pairs = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(pairs.len(), 8);
    assert_eq!(pairs["category"], category.to_string());
    assert_eq!(pairs["prefix"], prefix);
    assert_eq!(pairs["pn"], page.to_string());
    assert_eq!(pairs["rn"], "60");
    assert_eq!(pairs["httpsStatus"], "1");
    assert_eq!(pairs["plat"], "web_www");
    assert_eq!(pairs["from"], "");
    assert_eq!(pairs["reqId"].len(), 36);
    let headers = wire.to_ascii_lowercase();
    assert!(headers.contains("referer: https://www.kuwo.cn/singers\r\n"));
    for header in ["cookie:", "secret:", "authorization:"] {
        assert!(!headers.contains(header));
    }
    pairs["reqId"].to_string()
}

#[tokio::test]
async fn singer_list_sdk_and_provider_preserve_metadata_and_detail_identity() {
    for sdk in [false, true] {
        let mut f = setup(vec![json_response(&body(1, 1))]).await;
        let request = ArtistListRequest::new(20, 0);
        let result = if sdk {
            f.client.artists(&request).await
        } else {
            f.provider.artists(&request).await
        }
        .unwrap();
        assert!(f.provider.capabilities().contains(&Capability::ArtistList));
        let artist = &result.items[0];
        assert_eq!(artist.resource_ref.to_string(), "kuwo:1");
        assert_eq!(artist.id, "1");
        assert_eq!(artist.name, "Singer 1 & Friends");
        assert_eq!(artist.aliases, ["Alias"]);
        assert_eq!(artist.track_count, Some(23));
        assert_eq!(artist.album_count, Some(5));
        assert_eq!(artist.mv_count, Some(3));
        assert_eq!(artist.video_count, None);
        assert_eq!(artist.extensions["source_is_star"], 0);
        assert_eq!(artist.extensions["artist_fans"], 0);
        assert_eq!(result.pagination.total, Some(1));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        assert_eq!(
            result.pagination.extensions["catalog_scope"],
            "official_web_singers"
        );
        assert!(
            !result
                .pagination
                .extensions
                .contains_key("complete_snapshot")
        );
        let detail = super::super::parse_artist(
            &serde_json::to_vec(&json!({"code":200,"data":singer(1)})).unwrap(),
            "1",
        )
        .unwrap();
        assert_eq!(artist.resource_ref, detail.resource_ref);
        wire(&requests(&mut f, 1).await[0], 0, "", 1);
    }
}

#[tokio::test]
async fn singer_list_arbitrary_window_uses_at_most_three_pages_in_upstream_order() {
    let mut f = setup((1..=3).map(|p| json_response(&body(p, 200))).collect()).await;
    let result = f
        .provider
        .artists(&ArtistListRequest::new(100, 59))
        .await
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|a| a.id.clone())
            .collect::<Vec<_>>(),
        (60..=159).map(|id| id.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(result.pagination.total, Some(200));
    assert_eq!(result.pagination.next_offset, Some(159));
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 3);
    let mut ids = BTreeSet::new();
    for (index, request) in requests(&mut f, 3).await.iter().enumerate() {
        assert!(ids.insert(wire(request, 0, "", index as u32 + 1)));
    }
}

#[tokio::test]
async fn singer_list_tail_and_out_of_range_keep_the_upstream_total_without_seed_reads() {
    for (offset, count, page) in [(61, 2, 2), (63, 0, 2), (60_000, 0, 1001)] {
        let mut f = setup(vec![json_response(&body(page, 63))]).await;
        let result = f
            .client
            .artists(&ArtistListRequest::new(100, offset))
            .await
            .unwrap();
        assert_eq!(result.items.len(), count);
        assert_eq!(result.pagination.total, Some(63));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        wire(&requests(&mut f, 1).await[0], 0, "", page);
    }
}

#[test]
fn singer_list_explicit_empty_differs_from_missing_null_and_short_pages() {
    assert!(
        parse(&serde_json::to_vec(&body(1, 0)).unwrap(), 1)
            .unwrap()
            .items
            .is_empty()
    );
    for value in [
        json!({"code":200}),
        json!({"code":200,"data":null}),
        json!({"code":200,"data":{}}),
        json!({"code":200,"data":{"total":0}}),
        json!({"code":200,"data":{"total":0,"artistList":null}}),
        json!({"code":200,"data":{"artistList":[]}}),
        json!({"code":200,"data":{"total":61,"artistList":[]}}),
        json!({"code":200,"data":{"total":0,"artistList":[singer(1)]}}),
    ] {
        assert!(parse(&serde_json::to_vec(&value).unwrap(), 1).is_err());
    }
}

#[test]
fn singer_list_absent_metadata_remains_unknown() {
    let result = parse(
        &serde_json::to_vec(&json!({"code":200,"data":{"total":1,"artistList":[
            {"id":"42","name":"Minimal","pic":"https://untrusted.example/a.jpg"}
        ]}}))
        .unwrap(),
        1,
    )
    .unwrap();
    let artist = &result.items[0];
    assert_eq!(artist.id, "42");
    assert!(artist.aliases.is_empty());
    assert!(artist.avatar_url.is_none());
    assert_eq!(artist.track_count, None);
    assert_eq!(artist.album_count, None);
    assert_eq!(artist.mv_count, None);
    assert!(!artist.extensions.contains_key("artist_fans"));
    assert!(!artist.extensions.contains_key("source_is_star"));
}

#[test]
fn singer_list_rejects_invalid_identity_counts_and_non_music_content() {
    for (key, value) in [
        ("id", json!(0)),
        ("id", json!("0042")),
        ("name", json!("")),
        ("musicNum", json!(-1)),
        ("albumNum", json!("unknown")),
        ("mvNum", json!(1.5)),
        ("content_type", json!(1)),
    ] {
        let mut value_body = body(1, 1);
        value_body["data"]["artistList"][0][key] = value;
        assert!(parse(&serde_json::to_vec(&value_body).unwrap(), 1).is_err());
    }
}

#[tokio::test]
async fn singer_list_duplicate_identities_within_or_across_pages_fail_the_whole_window() {
    let mut within = body(1, 2);
    within["data"]["artistList"][1]["id"] = json!(1);
    let mut across = body(2, 120);
    across["data"]["artistList"][0]["id"] = json!(60);
    for responses in [vec![within], vec![body(1, 120), across]] {
        let count = responses.len();
        let mut f = setup(responses.iter().map(json_response).collect()).await;
        assert!(
            f.client
                .artists(&ArtistListRequest::new(100, 0))
                .await
                .is_err()
        );
        requests(&mut f, count).await;
    }
}

#[tokio::test]
async fn singer_list_changed_totals_do_not_return_a_partial_window() {
    let mut f = setup(vec![
        json_response(&body(1, 120)),
        json_response(&body(2, 121)),
    ])
    .await;
    assert!(
        f.provider
            .artists(&ArtistListRequest::new(100, 0))
            .await
            .is_err()
    );
    requests(&mut f, 2).await;
}

#[tokio::test]
async fn singer_list_http_mime_and_business_errors_are_not_empty_successes() {
    for reply in [
        response(500, "application/json", "", b"{}"),
        response(200, "text/html", "", b"{}"),
        json_response(&json!({"code":500,"data":{"total":0,"artistList":[]}})),
        response(200, "application/json", "", b"not JSON"),
    ] {
        let mut f = setup(vec![reply]).await;
        assert!(
            f.client
                .artists(&ArtistListRequest::new(20, 0))
                .await
                .is_err()
        );
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn singer_list_supported_category_pairs_match_the_official_combined_selector() {
    for (area, category, selector) in [
        (ArtistArea::All, ArtistCategory::All, 0),
        (ArtistArea::Chinese, ArtistCategory::Male, 1),
        (ArtistArea::Chinese, ArtistCategory::Female, 2),
        (ArtistArea::Chinese, ArtistCategory::Group, 3),
        (ArtistArea::JapaneseKorean, ArtistCategory::Male, 4),
        (ArtistArea::JapaneseKorean, ArtistCategory::Female, 5),
        (ArtistArea::JapaneseKorean, ArtistCategory::Group, 6),
        (ArtistArea::Western, ArtistCategory::Male, 7),
        (ArtistArea::Western, ArtistCategory::Female, 8),
        (ArtistArea::Western, ArtistCategory::Group, 9),
        (ArtistArea::Other, ArtistCategory::All, 10),
    ] {
        let mut f = setup(vec![json_response(&body(1, 0))]).await;
        let mut request = ArtistListRequest::new(20, 0);
        request.area = area;
        request.category = category;
        assert!(f.provider.artists(&request).await.unwrap().items.is_empty());
        wire(&requests(&mut f, 1).await[0], selector, "", 1);
    }
}

#[tokio::test]
async fn singer_list_japanese_korean_subclasses_preserve_cross_page_identity_in_sdk_and_provider() {
    for sdk in [false, true] {
        for (category, selector) in [
            (ArtistCategory::Male, 4),
            (ArtistCategory::Female, 5),
            (ArtistCategory::Group, 6),
        ] {
            // Synthetic field fixtures: no platform artists or account data are retained.
            let mut f = setup(vec![
                json_response(&body(1, 61)),
                json_response(&body(2, 61)),
            ])
            .await;
            let mut request = ArtistListRequest::new(2, 59);
            request.area = ArtistArea::JapaneseKorean;
            request.category = category;
            let result = if sdk {
                f.client.artists(&request).await
            } else {
                f.provider.artists(&request).await
            }
            .unwrap();
            assert_eq!(
                result
                    .items
                    .iter()
                    .map(|artist| artist.resource_ref.to_string())
                    .collect::<Vec<_>>(),
                ["kuwo:60", "kuwo:61"]
            );
            assert_eq!(result.pagination.total, Some(61));
            assert_eq!(result.pagination.next_offset, None);
            assert!(!result.pagination.has_more);
            assert_eq!(result.pagination.extensions["upstream_category"], selector);
            let calls = requests(&mut f, 2).await;
            wire(&calls[0], selector, "", 1);
            wire(&calls[1], selector, "", 2);
        }
    }
}

#[tokio::test]
async fn singer_list_initial_hash_retains_the_official_double_encoding() {
    for (initial, prefix) in [
        (None, ""),
        (Some(""), ""),
        (Some("a"), "A"),
        (Some("Z"), "Z"),
        (Some("#"), "%23"),
    ] {
        let mut f = setup(vec![json_response(&body(1, 0))]).await;
        let mut request = ArtistListRequest::new(20, 0);
        request.initial = initial.map(str::to_owned);
        f.client.artists(&request).await.unwrap();
        let raw = requests(&mut f, 1).await.remove(0);
        wire(&raw, 0, prefix, 1);
        if initial == Some("#") {
            assert!(raw.contains("prefix=%2523&"));
        }
    }
}

#[tokio::test]
async fn singer_list_unsupported_filters_and_invalid_windows_fail_before_network() {
    let mut f = setup(vec![]).await;
    let mut invalid = Vec::new();
    for (area, category) in [
        (ArtistArea::All, ArtistCategory::Male),
        (ArtistArea::Chinese, ArtistCategory::All),
        (ArtistArea::Western, ArtistCategory::All),
        (ArtistArea::Other, ArtistCategory::Female),
        (ArtistArea::Japanese, ArtistCategory::Male),
        (ArtistArea::Korean, ArtistCategory::Male),
        (ArtistArea::JapaneseKorean, ArtistCategory::All),
        (ArtistArea::HongKongTaiwan, ArtistCategory::Male),
    ] {
        let mut request = ArtistListRequest::new(20, 0);
        request.area = area;
        request.category = category;
        invalid.push(request);
    }
    for initial in ["AA", "中", "%23", "?", "A&B", " A"] {
        let mut request = ArtistListRequest::new(20, 0);
        request.initial = Some(initial.into());
        invalid.push(request);
    }
    for account in ["", "default", "selected"] {
        let mut request = ArtistListRequest::new(20, 0);
        request.account = Some(account.into());
        invalid.push(request);
    }
    let mut genre = ArtistListRequest::new(20, 0);
    genre.genre = ArtistGenre::Pop;
    invalid.push(genre);
    invalid.extend([
        ArtistListRequest::new(0, 0),
        ArtistListRequest::new(101, 0),
        ArtistListRequest::new(1, u32::MAX),
    ]);
    for request in invalid {
        assert_eq!(
            f.client.artists(&request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.artists(&request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(f.seen.try_recv().is_err());
}

#[tokio::test]
async fn singer_list_caller_scope_is_rejected_without_using_or_rotating_credentials() {
    let mut f = setup(vec![]).await;
    let credential =
        crate::client::native::tests::credential_fixture("42", "private-singer-session")
            .caller()
            .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller
            .artists(&ArtistListRequest::new(20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(f.seen.try_recv().is_err());
}

#[tokio::test]
async fn singer_list_does_not_send_or_replace_an_existing_www_tracking_session() {
    let mut f = setup(vec![json_response(&body(1, 0))]).await;
    *f.client.web_session.lock().await = Some(crate::client::KuwoWebSession {
        cookie_value: "private-www-tracking-cookie".into(),
        refresh_after: Instant::now() + Duration::from_secs(120),
    });
    f.provider
        .artists(&ArtistListRequest::new(20, 0))
        .await
        .unwrap();
    wire(&requests(&mut f, 1).await[0], 0, "", 1);
    assert_eq!(
        f.client
            .web_session
            .lock()
            .await
            .as_ref()
            .unwrap()
            .cookie_value,
        "private-www-tracking-cookie"
    );
}
