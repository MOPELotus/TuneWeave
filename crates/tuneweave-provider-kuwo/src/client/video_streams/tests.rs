use super::*;
use crate::{
    KuwoProvider,
    client::catalog::tests::{home, home_with, json_response, requests, response, setup},
};
use tuneweave_core::{MusicProvider, VideoResourceKind, VideoStreamRequest};

const URL: &str = "https://kw-bj.kuwo.cn/0123456789abcdef0123456789abcdef/1234abcd/ll/resource/m3/24/54/1234567890.mp4";
fn metadata(id: u64, play: serde_json::Value) -> serde_json::Value {
    json!({"code":200,"data":{"rid":id,"musicrid":format!("MUSIC_{id}"),"name":"Video","artist":"Artist","artistid":336,
        "duration":214,"hasmv":1,"online":1,"mvpayinfo":{"vid":8139776,"play":play},"mvPlayCnt":100}})
}
fn granted() -> serde_json::Value {
    json!({"code":200,"data":{"url":URL}})
}
fn request() -> VideoStreamRequest {
    VideoStreamRequest::new(VideoResourceKind::Mv, 1080)
}

#[test]
fn mv_urls_are_confined_to_the_observed_https_host_and_resource_paths() {
    assert_eq!(validate_url(URL).unwrap(), URL);
    assert!(validate_url(&URL.replace("/ll/", "/rc/").replace("/m3/", "/m2/")).is_ok());
    for bad in [
        URL.replace("https:", "http:"),
        URL.replace("kw-bj.kuwo.cn", "kw-bj.kuwo.cn.evil.test"),
        URL.replace("kw-bj.kuwo.cn", "other.kuwo.cn"),
        URL.replace("kw-bj.kuwo.cn", "user@kw-bj.kuwo.cn"),
        URL.replace("kw-bj.kuwo.cn", "kw-bj.kuwo.cn:444"),
        URL.replace("/ll/", "/unknown/"),
        URL.replace("/resource/", "/private/"),
        URL.replace("/m3/", "/m../"),
        URL.replace("/24/", "/%32%34/"),
        URL.replace("/24/", "/99/../24/"),
        URL.replace(".mp4", ".mp3"),
        URL.replace(".mp4", ".mp4/extra"),
        URL.replace("/54/", "//"),
        format!("{URL}?token=never-export"),
        format!("{URL}#fragment"),
        format!(" {URL}"),
        URL.replace("/ll/", "\\ll/"),
        URL.replace("1234abcd", "short"),
    ] {
        assert!(validate_url(&bad).is_err(), "unexpected URL acceptance");
    }
}

#[test]
fn only_explicit_endpoint_denials_become_unavailable_and_other_bad_data_is_an_error() {
    assert!(matches!(
        parse(&serde_json::to_vec(&granted()).unwrap()).unwrap(),
        Outcome::Allowed(_)
    ));
    for code in [-1, -1001] {
        assert!(
            matches!(parse(&serde_json::to_vec(&json!({"code":code,"data":null})).unwrap()).unwrap(),Outcome::Denied(actual) if actual==code)
        );
    }
    for value in [
        json!({}),
        json!({"code":200,"data":null}),
        json!({"code":200,"data":{}}),
        json!({"code":200,"data":{"url":""}}),
        json!({"code":-1,"data":{"url":URL}}),
        json!({"code":500,"data":null,"msg":"never-export"}),
    ] {
        let error = parse(&serde_json::to_vec(&value).unwrap()).err().unwrap();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("never-export"));
    }
}

#[tokio::test]
async fn granted_playback_checks_current_metadata_then_requests_only_mv_mid() {
    let mut fixture = setup(vec![
        home(),
        json_response(&metadata(215252, json!(0))),
        json_response(&granted()),
    ])
    .await;
    let stream = fixture
        .provider
        .video_stream(
            "215252",
            &VideoStreamRequest::new(VideoResourceKind::Mv, 4320),
        )
        .await
        .unwrap();
    assert!(stream.available);
    assert_eq!(stream.url.as_deref(), Some(URL));
    assert_eq!(stream.video_ref.to_string(), "kuwo:215252");
    assert_eq!(stream.format.as_deref(), Some("mp4"));
    assert_eq!(stream.requested_resolution, 4320);
    assert!(
        stream.actual_resolution.is_none()
            && stream.width.is_none()
            && stream.height.is_none()
            && stream.codec.is_none()
    );
    assert!(stream.size.is_none() && stream.expires_at.is_none() && stream.fee.is_none());
    assert!(stream.headers.is_empty());
    assert_eq!(stream.platform_code, Some(200));
    assert_eq!(stream.duration_ms, Some(214000));
    assert_eq!(
        stream.extensions["resolution_selection"],
        "platform_default"
    );
    let seen = requests(&mut fixture, 3).await;
    assert!(seen[1].starts_with("GET /api/www/music/musicInfo?"));
    let call = &seen[2];
    let url = Url::parse(&format!(
        "https://www.kuwo.cn{}",
        call.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(url.path(), "/api/v1/www/music/playUrl");
    let params = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(params["mid"], "215252");
    assert_eq!(params["type"], "mv");
    assert_eq!(params["plat"], "web_www");
    assert_eq!(params["from"], "");
    assert_eq!(params["httpsStatus"], "1");
    assert_eq!(params.len(), 6);
    assert!(call.contains("secret: ") && call.contains("cookie: "));
    assert!(
        !params.contains_key("vid")
            && !params.contains_key("br")
            && !params.contains_key("resolution")
    );
}

#[tokio::test]
async fn known_denials_stop_before_url_requests_and_unknown_permission_is_not_a_grant() {
    for mode in 0..5 {
        let mut info = metadata(215252, json!(0));
        match mode {
            0 => info["data"]["online"] = json!(0),
            1 => info["data"]["disable"] = json!(1),
            2 => info["data"]["mvpayinfo"]["play"] = json!(1),
            3 => info["data"]["mvpayinfo"]["play"] = json!(null),
            _ => info["data"]["mvpayinfo"]["play"] = json!(2),
        }
        let mut fixture = setup(vec![home(), json_response(&info)]).await;
        let result = fixture.provider.video_stream("215252", &request()).await;
        if mode < 3 {
            let stream = result.unwrap();
            assert!(!stream.available && stream.url.is_none());
            assert!(stream.platform_code.is_none() && stream.fee.is_none());
            assert_eq!(
                stream.extensions["unavailable_reason"],
                ["offline", "disabled", "permission_denied"][mode]
            );
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::UpstreamError);
        }
        requests(&mut fixture, 2).await;
    }
}

#[tokio::test]
async fn batches_preserve_duplicates_and_recheck_permissions_on_the_next_call() {
    let mut fixture = setup(vec![
        home(),
        json_response(&metadata(215252, json!(0))),
        json_response(&granted()),
        json_response(&metadata(550531865, json!(1))),
        json_response(&metadata(215252, json!(1))),
    ])
    .await;
    let ids = vec!["215252".into(), "550531865".into(), "215252".into()];
    let streams = fixture
        .provider
        .video_streams(&ids, &request())
        .await
        .unwrap();
    assert_eq!(
        streams
            .iter()
            .map(|v| v.video_ref.id().to_owned())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(streams[0], streams[2]);
    assert!(streams[0].available && !streams[1].available);
    let next = fixture
        .provider
        .video_stream("215252", &request())
        .await
        .unwrap();
    assert!(!next.available);
    requests(&mut fixture, 5).await;
    let mut max = setup(vec![
        home(),
        json_response(&metadata(215252, json!(0))),
        json_response(&granted()),
    ])
    .await;
    let many = max
        .provider
        .video_streams(&vec!["215252".into(); 100], &request())
        .await
        .unwrap();
    assert_eq!(many.len(), 100);
    assert!(many.iter().all(|v| v.available));
    requests(&mut max, 3).await;
}

#[tokio::test]
async fn every_batch_input_is_validated_before_io_and_late_failures_return_no_partial_batch() {
    let mut fixture = setup(vec![]).await;
    for ids in [
        vec![],
        vec!["215252".into(); 101],
        vec!["215252".into(), "0215252".into()],
        vec!["215252&host=evil.test".into()],
    ] {
        assert!(
            fixture
                .provider
                .video_streams(&ids, &request())
                .await
                .is_err()
        );
    }
    for r in [
        VideoStreamRequest::new(VideoResourceKind::Mv, 0),
        VideoStreamRequest::new(VideoResourceKind::Video, 1080),
        VideoStreamRequest {
            account: Some("private".into()),
            ..request()
        },
    ] {
        assert!(fixture.provider.video_stream("215252", &r).await.is_err());
    }
    requests(&mut fixture, 0).await;
    let mut late = setup(vec![
        home(),
        json_response(&metadata(215252, json!(0))),
        json_response(&granted()),
        response(200, "text/html", "", b"{}"),
    ])
    .await;
    assert!(
        late.provider
            .video_streams(&["215252".into(), "3362686".into()], &request())
            .await
            .is_err()
    );
    requests(&mut late, 4).await;
}

#[tokio::test]
async fn url_signature_refresh_is_bounded_and_uses_the_new_cookie() {
    for forbidden in [
        response(403, "application/json", "", b"{}"),
        json_response(&json!({"success":false,"message":"The request is illegal!"})),
    ] {
        for success in [true, false] {
            let mut fixture = setup(vec![
                home(),
                json_response(&metadata(215252, json!(0))),
                forbidden.clone(),
                home_with("NewTrackingCookie12345678"),
                if success {
                    json_response(&granted())
                } else {
                    forbidden.clone()
                },
            ])
            .await;
            assert_eq!(
                fixture
                    .provider
                    .video_stream("215252", &request())
                    .await
                    .is_ok(),
                success
            );
            let seen = requests(&mut fixture, 5).await;
            assert!(seen[4].contains("NewTrackingCookie12345678"));
            assert!(!seen[4].contains("anonymousCatalogueCookie123456"));
        }
    }
}

#[tokio::test]
async fn endpoint_denials_are_explicit_but_transport_and_untrusted_urls_remain_errors() {
    for code in [-1, -1001] {
        let mut fixture = setup(vec![
            home(),
            json_response(&metadata(215252, json!(0))),
            json_response(&json!({"code":code,"data":null,"msg":"never-export"})),
        ])
        .await;
        let stream = fixture
            .provider
            .video_stream("215252", &request())
            .await
            .unwrap();
        assert!(!stream.available && stream.url.is_none());
        assert_eq!(stream.platform_code, Some(code));
        assert!(
            !serde_json::to_string(&stream)
                .unwrap()
                .contains("never-export")
        );
        requests(&mut fixture, 3).await;
    }
    for bad in [response(429,"application/json","Retry-After: 2\r\n",b"{}"),response(302,"application/json","Location: https://example.test/\r\n",b"{}"),response(200,"text/html","",b"{}"),json_response(&json!({"code":200,"data":{"url":"https://example.test/never-export.mp4"}})),b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".to_vec()] {
        let mut fixture=setup(vec![home(),json_response(&metadata(215252,json!(0))),bad]).await;
        let error=fixture.provider.video_stream("215252",&request()).await.unwrap_err();assert!(!format!("{error:?}").contains("never-export"));requests(&mut fixture,3).await;
    }
}

#[tokio::test]
#[ignore = "Official anonymous MV authorization, URL and media HEAD; no real account or media body"]
async fn live_authorized_mv_and_denied_mv_keep_permissions_and_unknown_resolution() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    let streams = provider
        .video_streams(
            &["215252".into(), "550531865".into(), "215252".into()],
            &request(),
        )
        .await
        .unwrap();
    assert!(streams[0].available && !streams[1].available);
    assert_eq!(streams[0], streams[2]);
    assert!(streams[1].url.is_none());
    let stream = &streams[0];
    assert!(stream.actual_resolution.is_none() && stream.expires_at.is_none());
    let url = validate_url(stream.url.as_deref().unwrap()).unwrap();
    let response = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap()
        .head(url)
        .header(REFERER, WEB_REFERER)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "video/mp4"
    );
    assert!(
        response
            .headers()
            .get(CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
}
