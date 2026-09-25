use super::*;
use crate::{
    KuwoProvider,
    client::catalog::tests::{home, home_with, json_response, requests, response, setup},
};
use tuneweave_core::{ArtistVideoListRequest, MusicProvider, VideoDetailRequest, VideoKind};

fn body() -> serde_json::Value {
    json!({"code":200,"data":{"rid":550531865,"musicrid":"MUSIC_550531865","name":"Video&nbsp;Title",
        "artist":"First&amp;Name&Second","artistid":336,"duration":214,"hasmv":1,"online":1,"content_type":"0",
        "mvpayinfo":{"vid":18149845,"play":1,"down":1},"mvPlayCnt":43864,"releaseDate":"2026-03-25",
        "pic":"https://img2.kuwo.cn/star/albumcover/500/a.jpg","albuminfo":"Album biography, not MV description",
        "upPcStr":"never-export","mvUpPcStr":"never-export","opaque":{"sid":"never-export"}}})
}
fn parsed(value: &serde_json::Value) -> Result<VideoDetail> {
    parse(&serde_json::to_vec(value).unwrap(), "550531865")
}
fn request() -> VideoDetailRequest {
    VideoDetailRequest::new(VideoResourceKind::Mv)
}

#[test]
fn mv_detail_keeps_actual_identity_and_catalogue_codes_without_playback_grants() {
    let detail = parsed(&body()).unwrap();
    let video = &detail.video;
    assert_eq!(video.resource_ref.to_string(), "kuwo:550531865");
    assert_eq!(video.title, "Video Title");
    assert_eq!(video.creators[0].name, "First&Name");
    assert!(video.creators[1].resource_ref.is_none());
    assert_eq!(video.duration_ms, Some(214000));
    assert_eq!(video.play_count, Some(43864));
    assert!(
        video.description.is_empty() && video.published_at.is_none() && video.subscribed.is_none()
    );
    assert!(detail.resolutions.is_empty());
    assert_eq!(video.extensions["source_track_release_date"], "2026-03-25");
    assert_eq!(video.extensions["mv_pay_info"]["vid"], "18149845");
    assert_eq!(video.extensions["mv_pay_info"]["play"], 1);
    let text = serde_json::to_string(&detail).unwrap();
    for absent in ["never-export", "Album biography", "playable"] {
        assert!(!text.contains(absent));
    }
    let mut unknown = body();
    for key in ["duration", "mvPlayCnt", "online", "pic", "releaseDate"] {
        unknown["data"].as_object_mut().unwrap().remove(key);
    }
    unknown["data"]["mvpayinfo"]
        .as_object_mut()
        .unwrap()
        .remove("play");
    let video = parsed(&unknown).unwrap().video;
    assert!(video.duration_ms.is_none() && video.play_count.is_none() && video.cover_url.is_none());
    assert!(
        !video.extensions["mv_pay_info"]
            .as_object()
            .unwrap()
            .contains_key("play")
    );
    assert!(!video.extensions.contains_key("disable"));
}

#[test]
fn no_mv_is_distinct_from_malformed_or_foreign_metadata() {
    assert_eq!(
        parsed(&json!({"code":-1,"data":null})).unwrap_err().code,
        ErrorCode::ResourceNotFound
    );
    let mut absent = body();
    absent["data"]["hasmv"] = json!(0);
    assert_eq!(
        parsed(&absent).unwrap_err().code,
        ErrorCode::ResourceNotFound
    );
    for (key, bad) in [
        ("rid", json!(18149845)),
        ("musicrid", json!("MUSIC_18149845")),
        ("hasmv", json!(null)),
        ("hasmv", json!(2)),
        ("content_type", json!(4)),
        ("ad_type", json!("ad")),
        ("duration", json!(u64::MAX)),
        ("mvPlayCnt", json!("01")),
        ("online", json!(2)),
        ("disable", json!(2)),
        ("artistid", json!("0336")),
        ("name", json!("\u{0000}")),
    ] {
        let mut value = body();
        value["data"][key] = bad;
        assert_eq!(
            parsed(&value).unwrap_err().code,
            ErrorCode::UpstreamError,
            "{key}"
        );
    }
    for (key, bad) in [
        ("vid", json!(0)),
        ("vid", json!(null)),
        ("play", json!(-1)),
        ("play", json!(true)),
    ] {
        let mut value = body();
        value["data"]["mvpayinfo"][key] = bad;
        assert_eq!(parsed(&value).unwrap_err().code, ErrorCode::UpstreamError);
    }
    let error = parsed(&json!({"code":400,"msg":"never-export","data":null})).unwrap_err();
    assert!(!format!("{error:?}").contains("never-export"));
}

#[tokio::test]
async fn details_and_stats_use_music_mid_and_a_shared_anonymous_signed_session() {
    let mut fixture = setup(vec![home(), json_response(&body()), json_response(&body())]).await;
    let detail = fixture
        .provider
        .video("550531865", &request())
        .await
        .unwrap();
    let stats = fixture
        .provider
        .video_stats("550531865", &request())
        .await
        .unwrap();
    assert_eq!(stats.video_ref, detail.video.resource_ref);
    assert_eq!(stats.view_count, Some(43864));
    assert!(
        stats.liked.is_none()
            && stats.favorited.is_none()
            && stats.like_count.is_none()
            && stats.comment_count.is_none()
    );
    let seen = requests(&mut fixture, 3).await;
    for wire in &seen[1..] {
        let url = Url::parse(&format!(
            "https://www.kuwo.cn{}",
            wire.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        assert_eq!(url.path(), "/api/www/music/musicInfo");
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query["mid"], "550531865");
        assert_eq!(query["ip"], "");
        assert_eq!(query["cip"], "");
        assert_eq!(query.len(), 7);
        assert!(wire.to_ascii_lowercase().contains("secret:"));
        assert!(wire.to_ascii_lowercase().contains("cookie:"));
        assert!(wire.contains("referer: https://www.kuwo.cn/mvplay/550531865"));
        assert!(!wire.contains("18149845"));
    }
}

#[tokio::test]
async fn rejected_signatures_refresh_once_and_use_only_the_new_tracking_cookie() {
    for success in [false, true] {
        let forbidden = response(403, "application/json", "", b"{}");
        let mut fixture = setup(vec![
            home(),
            forbidden.clone(),
            home_with("NewAnonymousCookie123456"),
            if success {
                json_response(&body())
            } else {
                forbidden
            },
        ])
        .await;
        assert_eq!(
            fixture
                .provider
                .video("550531865", &request())
                .await
                .is_ok(),
            success
        );
        let seen = requests(&mut fixture, 4).await;
        assert!(seen[3].contains("NewAnonymousCookie123456"));
        assert!(!seen[3].contains("anonymousCatalogueCookie123456"));
    }
}

#[tokio::test]
async fn detail_modes_accounts_and_invalid_ids_stop_before_any_request() {
    let mut fixture = setup(vec![]).await;
    for id in [
        "",
        "0",
        "0550531865",
        "550531865&other=1",
        "18446744073709551616",
    ] {
        assert!(fixture.provider.video(id, &request()).await.is_err());
        assert!(fixture.provider.video_stats(id, &request()).await.is_err());
    }
    for request in [
        VideoDetailRequest::new(VideoResourceKind::Video),
        VideoDetailRequest {
            account: Some("private".into()),
            ..request()
        },
    ] {
        assert!(fixture.provider.video("550531865", &request).await.is_err());
        assert!(
            fixture
                .provider
                .video_stats("550531865", &request)
                .await
                .is_err()
        );
    }
    requests(&mut fixture, 0).await;
}

#[tokio::test]
async fn terminal_mv_http_and_body_failures_never_fall_back_to_another_id() {
    for bad in [response(429,"application/json","Retry-After: 3\r\n",b"{}"),response(302,"application/json","Location: https://example.test/\r\n",b"{}"),response(200,"text/html","",b"{}"),response(200,"application/json","",b"{}"),response(200,"application/json","",b"{\"code\":400,\"msg\":\"never-export\",\"data\":null}"),b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".to_vec()] {
        let mut fixture=setup(vec![home(),bad]).await;
        let error=fixture.provider.video("550531865",&request()).await.unwrap_err();assert!(!format!("{error:?}").contains("never-export"));requests(&mut fixture,2).await;
    }
}

#[tokio::test]
#[ignore = "Current official MV catalogue, metadata and statistics; no account or playback request"]
async fn live_artist_mv_reference_resolves_to_matching_detail_stats_and_track() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    let page = provider
        .artist_videos(
            "336",
            &ArtistVideoListRequest {
                kind: VideoKind::Mv,
                ..ArtistVideoListRequest::new(1, 0)
            },
        )
        .await
        .unwrap();
    let entry = &page.items[0];
    let detail = provider.video(&entry.id, &request()).await.unwrap();
    assert_eq!(detail.video.resource_ref, entry.resource_ref);
    assert_eq!(detail.video.title, entry.title);
    let stats = provider.video_stats(&entry.id, &request()).await.unwrap();
    assert_eq!(stats.video_ref, entry.resource_ref);
    assert!(stats.view_count.is_some());
    let track = provider.track(&entry.id, None).await.unwrap();
    assert_eq!(track.mv_ref.as_ref(), Some(&entry.resource_ref));
    assert_ne!(detail.video.extensions["mv_pay_info"]["vid"], entry.id);
    assert!(detail.resolutions.is_empty());
}
