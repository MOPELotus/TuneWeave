use super::*;
use crate::{
    KuwoProvider,
    client::catalog::tests::{home, json_response, requests, response, setup},
};
use tuneweave_core::{ArtistVideoListRequest, MusicProvider, VideoKind};

fn profile(total: u64) -> serde_json::Value {
    json!({"code":200,"data":{"id":336,"name":"Artist","aartist":"Alias","info":"Biography","mvNum":total}})
}
fn body(page: u32, total: u64) -> serde_json::Value {
    let start = u64::from(page - 1) * 20;
    let mvlist:Vec<_>=(start..total.min(start+20)).map(|n|json!({
        "id":(n+1).to_string(),"name":format!("Video {}&nbsp;(Live)",n+1),"artist":"Artist","artistid":336,
        "duration":214,"mvPlayCnt":1234,"online":"1","pic":"https://img1.kuwo.cn/wmvpic/324/a.jpg",
        "vid":18149845,"opaque":{"sid":"never-export"},"playurl":"https://example.test/not-a-grant"
    })).collect();
    json!({"code":200,"data":{"total":total.to_string(),"mvlist":mvlist}})
}
fn request(limit: u32, offset: u32) -> ArtistVideoListRequest {
    ArtistVideoListRequest {
        kind: VideoKind::Mv,
        ..ArtistVideoListRequest::new(limit, offset)
    }
}
fn parsed(value: &serde_json::Value, page: u32) -> Result<ArtistPage<Video>> {
    let artist = parse_artist(&serde_json::to_vec(&profile(0)).unwrap(), "336").unwrap();
    parse_mvs(&serde_json::to_vec(value).unwrap(), &artist, page)
}

#[test]
fn catalogue_keeps_music_id_credits_and_offline_rows_without_inventing_permissions() {
    let mut value = body(1, 2);
    value["data"]["mvlist"][1]["online"] = json!("0");
    let result = parsed(&value, 1).unwrap();
    assert_eq!(result.items.len(), 2);
    let first = &result.items[0];
    assert_eq!(first.resource_ref.to_string(), "kuwo:1");
    assert_eq!(first.title, "Video 1 (Live)");
    assert_eq!(first.duration_ms, Some(214000));
    assert_eq!(first.play_count, Some(1234));
    assert_eq!(first.creators[0].resource_ref.as_ref().unwrap().id(), "336");
    assert_eq!(first.extensions["source_track_id"], "1");
    assert_eq!(first.extensions["kind"], "mv");
    assert!(first.subscribed.is_none() && first.published_at.is_none());
    assert_eq!(result.items[1].extensions["online"], 0);
    let encoded = serde_json::to_string(&result.items).unwrap();
    for absent in ["never-export", "not-a-grant", "18149845", "playable"] {
        assert!(!encoded.contains(absent));
    }
    value["data"]["mvlist"][0]["artist"] = json!("Guest&amp;Name&Alias");
    value["data"]["mvlist"][0]["artistid"] = json!(999);
    let collab = parsed(&value, 1).unwrap();
    assert_eq!(collab.items[0].creators[0].name, "Guest&Name");
    assert!(collab.items[0].creators[1].resource_ref.is_none());
    for key in ["duration", "mvPlayCnt", "online", "pic"] {
        value["data"]["mvlist"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
    }
    let unknown = parsed(&value, 1).unwrap();
    assert!(
        unknown.items[0].duration_ms.is_none()
            && unknown.items[0].play_count.is_none()
            && unknown.items[0].cover_url.is_none()
    );
    assert!(!unknown.items[0].extensions.contains_key("online"));
}

#[test]
fn malformed_mv_identity_counts_and_foreign_credits_are_terminal() {
    for (key, bad) in [
        ("id", json!("01")),
        ("id", json!(0)),
        ("artistid", json!(999)),
        ("artist", json!("")),
        ("duration", json!(u64::MAX)),
        ("mvPlayCnt", json!(-1)),
        ("online", json!(2)),
        ("name", json!("\u{0000}")),
    ] {
        let mut value = body(1, 1);
        value["data"]["mvlist"][0][key] = bad;
        assert!(parsed(&value, 1).is_err(), "{key}");
    }
    for value in [
        json!({}),
        json!({"code":200,"data":{}}),
        json!({"code":200,"data":{"total":1,"mvlist":[]}}),
        json!({"code":200,"data":{"total":"-1","mvlist":[]}}),
        json!({"code":2001,"data":null}),
    ] {
        assert_eq!(
            parsed(&value, 1).err().unwrap().code,
            ErrorCode::UpstreamError
        );
    }
    assert!(parsed(&body(1, 0), 1).unwrap().items.is_empty());
    assert!(parsed(&body(1, 1), 0).is_err());
}

#[tokio::test]
async fn maximum_mv_window_reuses_verified_profile_and_fixed_unsigned_pages() {
    assert_eq!(
        UnsignedArtistEndpoint::Mvs.target().0,
        "https://wapi.kuwo.cn/api/www/artist/artistMv"
    );
    let mut responses = vec![home(), json_response(&profile(162))];
    for page in [1, 3, 4, 5, 6, 7, 8] {
        responses.push(json_response(&body(page, 162)));
    }
    let mut fixture = setup(responses).await;
    let result = fixture
        .provider
        .artist_videos("336", &request(100, 59))
        .await
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|v| v.id.clone())
            .collect::<Vec<_>>(),
        (60..160).map(|id| id.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(result.pagination.total, Some(162));
    assert_eq!(result.pagination.next_offset, Some(159));
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 7);
    let seen = requests(&mut fixture, 9).await;
    assert!(seen[1].to_ascii_lowercase().contains("secret:"));
    for (wire, pn) in seen[2..].iter().zip([1, 3, 4, 5, 6, 7, 8]) {
        let url = Url::parse(&format!(
            "https://wapi.kuwo.cn{}",
            wire.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        assert_eq!(url.path(), "/api/www/artist/artistMv");
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query["artistid"], "336");
        assert_eq!(query["pn"], pn.to_string());
        assert_eq!(query["rn"], "20");
        assert_eq!(query["plat"], "web_www");
        assert_eq!(query["from"], "");
        assert_eq!(query["httpsStatus"], "1");
        assert_eq!(query.len(), 7);
        assert!(!wire.to_ascii_lowercase().contains("cookie:"));
        assert!(!wire.to_ascii_lowercase().contains("secret:"));
    }
}

#[tokio::test]
async fn end_and_empty_windows_keep_the_actual_total_and_offline_positions() {
    for (total, offset, count, pages) in [
        (436, 421, 15, vec![1, 22]),
        (23, 19, 4, vec![1, 2]),
        (23, 80, 0, vec![1]),
        (0, 0, 0, vec![1]),
    ] {
        let mut responses = vec![home(), json_response(&profile(total))];
        for pn in &pages {
            let mut value = body(*pn, total);
            if let Some(first) = value["data"]["mvlist"].as_array_mut().unwrap().first_mut() {
                first["online"] = json!("0");
            }
            responses.push(json_response(&value));
        }
        let mut fixture = setup(responses).await;
        let result = fixture
            .provider
            .artist_videos("336", &request(20, offset))
            .await
            .unwrap();
        assert_eq!(result.items.len(), count);
        assert_eq!(result.pagination.total, Some(total));
        assert!(!result.pagination.has_more);
        if count > 0 {
            assert_eq!(result.items[0].id, (offset + 1).to_string());
        }
        requests(&mut fixture, 2 + pages.len()).await;
    }
}

#[tokio::test]
async fn drift_and_duplicate_or_foreign_pages_never_return_partial_video_results() {
    for fault in 0..5 {
        let mut second = body(2, 23);
        match fault {
            0 => second = body(2, 24),
            1 => second["data"]["mvlist"][0]["id"] = json!("1"),
            2 => second["data"]["mvlist"][0]["artistid"] = json!(337),
            3 => {
                second["data"]["mvlist"].as_array_mut().unwrap().pop();
            }
            _ => {}
        }
        let mut responses = vec![
            home(),
            json_response(&profile(if fault == 4 { 24 } else { 23 })),
            json_response(&body(1, 23)),
        ];
        if fault != 4 {
            responses.push(json_response(&second));
        }
        let count = responses.len();
        let mut fixture = setup(responses).await;
        assert!(
            fixture
                .provider
                .artist_videos("336", &request(20, 19))
                .await
                .is_err()
        );
        requests(&mut fixture, count).await;
    }
}

#[tokio::test]
async fn unsupported_video_modes_and_accounts_stop_before_io() {
    let mut fixture = setup(vec![]).await;
    for id in ["", "0", "0336", "336&x=1", "18446744073709551616"] {
        assert!(
            fixture
                .provider
                .artist_videos(id, &request(20, 0))
                .await
                .is_err()
        );
    }
    let mut bad = vec![
        request(0, 0),
        request(101, 0),
        request(1, u32::MAX),
        ArtistVideoListRequest::new(20, 0),
    ];
    let mut cursor = request(20, 0);
    cursor.cursor = Some("opaque".into());
    bad.push(cursor);
    let mut order = request(20, 0);
    order.order = Some("hot".into());
    bad.push(order);
    let mut account = request(20, 0);
    account.account = Some("private".into());
    bad.push(account);
    for request in bad {
        assert!(
            fixture
                .provider
                .artist_videos("336", &request)
                .await
                .is_err()
        );
    }
    requests(&mut fixture, 0).await;
    let mut wrong = profile(1);
    wrong["data"]["id"] = json!(337);
    let mut fixture = setup(vec![home(), json_response(&wrong)]).await;
    assert!(
        fixture
            .provider
            .artist_videos("336", &request(20, 0))
            .await
            .is_err()
    );
    requests(&mut fixture, 2).await;
}

#[tokio::test]
async fn unsigned_mv_failures_do_not_refresh_the_signed_session_or_follow_redirects() {
    for wire in [
        response(403, "application/json", "", b"{}"),
        response(429, "application/json", "Retry-After: 2\r\n", b"{}"),
        response(
            302,
            "application/json",
            "Location: https://example.test/\r\n",
            b"{}",
        ),
        response(200, "text/html", "", b"{}"),
        response(
            200,
            "application/json",
            "",
            b"{\"code\":2001,\"data\":null,\"msg\":\"never-export\"}",
        ),
    ] {
        let mut fixture = setup(vec![home(), json_response(&profile(1)), wire]).await;
        let error = fixture
            .provider
            .artist_videos("336", &request(20, 0))
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("never-export"));
        requests(&mut fixture, 3).await;
    }
}

#[tokio::test]
#[ignore = "Current official public artist MV catalogue; no account or media requests"]
async fn live_artist_mv_catalogue_end_window_and_music_identity() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    let artist = provider.artist("336", None).await.unwrap();
    let total = artist.mv_count.unwrap();
    assert!(total > 20);
    let page = provider
        .artist_videos("336", &request(20, u32::try_from(total - 15).unwrap()))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 15);
    assert_eq!(page.pagination.total, Some(total));
    assert!(!page.pagination.has_more);
    for video in &page.items {
        assert_eq!(video.extensions["source_track_id"], video.id);
        assert!(video.subscribed.is_none());
    }
    let beyond = provider
        .artist_videos("336", &request(20, u32::try_from(total + 20).unwrap()))
        .await
        .unwrap();
    assert!(beyond.items.is_empty());
    assert_eq!(beyond.pagination.total, Some(total));
}
