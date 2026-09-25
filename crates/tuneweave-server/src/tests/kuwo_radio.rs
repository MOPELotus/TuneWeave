//! Native FM is a broadcast identity, not a music RID or a programme recording.
use super::*;
use tuneweave_provider_kuwo::{KuwoConfig, KuwoProvider};

fn app(config: KuwoConfig) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(KuwoProvider::new(config).unwrap())
        .unwrap();
    build_router(AppState::new(registry, Platform::Kuwo))
}

#[tokio::test]
async fn kuwo_native_fm_http_rejects_accounts_music_ids_navigation_and_mixed_filters_before_io() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let router = app(KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    });
    for path in [
        "/v1/radio/taxonomy?platform=kuwo&account=default",
        "/v1/radio/stations?platform=kuwo&account=personal",
        "/v1/radio/stations?platform=kuwo&category_id=3&region_id=7",
        "/v1/radio/stations?platform=kuwo&category_id=99",
        "/v1/radio/stations?platform=kuwo&last_id=359&score=10",
        "/v1/radio/stations/kuwo:track:359",
        "/v1/radio/stations/kuwo:fm:0359",
        "/v1/radio/stations/kuwo:fm:359?account=personal",
    ] {
        let (status, body) = json_response_from(router.clone(), path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert_eq!(body["error"]["code"], "invalid_request");
        assert_eq!(
            guard.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[tokio::test]
#[ignore = "official anonymous Kuwo FM metadata only; no playlist, HLS manifest or audio is fetched"]
async fn live_kuwo_native_fm_http_and_uni_preserve_broadcast_identity_without_media_fetches() {
    let router = app(KuwoConfig::default());
    let (status, taxonomy) =
        json_response_from(router.clone(), "/v1/radio/taxonomy?platform=kuwo").await;
    assert_eq!(status, StatusCode::OK, "{taxonomy}");
    assert!(
        taxonomy["data"]["regions"]
            .as_array()
            .is_some_and(|items| items.iter().any(|v| v["id"] == "7"))
    );
    assert!(
        taxonomy["data"]["categories"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| !["95", "96", "97", "98", "99"].contains(&entry["id"].as_str().unwrap()))
    );
    let (status, list) = json_response_from(
        router.clone(),
        "/v1/radio/stations?platform=kuwo&region_id=7&limit=2&offset=0",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let stations = list["data"].as_array().unwrap();
    assert!(!stations.is_empty() && stations.len() <= 2);
    assert_eq!(list["meta"]["pagination"]["offset"], 0);
    // The retired service lists stations whose detail response has no data
    // (observed for 361). Use the independently verified metadata sample;
    // presence in the directory alone does not prove detail availability.
    let reference = stations
        .iter()
        .find(|station| station["ref"] == "kuwo:fm:359")
        .expect("the verified FM metadata sample is absent from the directory")["ref"]
        .as_str()
        .unwrap();
    assert!(reference.starts_with("kuwo:fm:"));
    let channel = reference.strip_prefix("kuwo:fm:").unwrap();
    let expected_url = format!("https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_{channel}.m3u8");
    let (status, detail) =
        json_response_from(router.clone(), &format!("/v1/radio/stations/{reference}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["data"]["ref"], reference);
    assert_eq!(detail["data"]["stream_url"], expected_url);
    assert_eq!(
        detail["data"]["extensions"]["stream_validation"],
        "official_url_metadata_only"
    );
    let (status, materialized) = json_request_from(
        router.clone(),
        Method::POST,
        "/v1/uni/materialize/items",
        Some(json!({"items":[{"ref":reference,"kind":"radio_station"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{materialized}");
    let item = materialized["data"]["items"][0].clone();
    assert_eq!(item["kind"], "radio_station");
    assert!(
        !item.to_string().contains(".m3u8"),
        "Uni snapshots must fetch current station URLs on playback"
    );
    let (status, stream) = json_request_from(
        router,
        Method::POST,
        "/v1/uni/items/stream",
        Some(json!({"item":item,"quality":"auto","fallback":false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stream}");
    assert_eq!(stream["data"]["source_ref"], reference);
    assert_eq!(stream["data"]["stream"]["resolved_track"], reference);
    assert_eq!(stream["data"]["stream"]["url"], expected_url);
    assert_eq!(stream["data"]["extensions"]["transport"], "live_radio");
    assert!(stream["data"]["stream"]["duration_ms"].is_null());
    assert!(stream["data"]["stream"]["bitrate"].is_null());
}
