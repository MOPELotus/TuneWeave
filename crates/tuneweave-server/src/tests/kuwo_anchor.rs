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
async fn kuwo_anchor_http_rejects_cross_kind_ids_accounts_and_invalid_pages_before_io() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let router = app(KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    });
    for path in [
        "/v1/podcasts/kuwo:9240675",
        "/v1/podcasts/kuwo:anchor:09240675",
        "/v1/podcasts/kuwo:anchor:9240675?account=default",
        "/v1/podcasts/kuwo:anchor:9240675/episodes?limit=101",
        "/v1/podcasts/kuwo:anchor:9240675/episodes?account=personal",
        "/v1/episodes/kuwo:64403133",
        "/v1/episodes/kuwo:anchor:64403133",
        "/v1/episodes/kuwo:episode:64403133?account=default",
    ] {
        let (status, body) = json_response_from(router.clone(), path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert_eq!(
            guard.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[tokio::test]
#[ignore = "official anonymous anchor metadata, Uni materialization and playback URL only; no audio fetched"]
async fn live_kuwo_anchor_http_uni_preserve_episode_album_and_audio_identity() {
    let router = app(KuwoConfig::default());
    let (status, list) = json_response_from(
        router.clone(),
        "/v1/podcasts/kuwo:anchor:9240675/episodes?limit=3&ascending=true",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["ref"] == "kuwo:episode:64403133")
    );
    let reference = "kuwo:episode:64403133";
    let (status, detail) =
        json_response_from(router.clone(), &format!("/v1/episodes/{reference}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["data"]["podcast_ref"], "kuwo:anchor:9240675");
    assert_eq!(detail["data"]["audio"]["ref"], "kuwo:64403133");
    let (status, materialized) = json_request_from(
        router.clone(),
        Method::POST,
        "/v1/uni/materialize/items",
        Some(json!({"items":[{"ref":reference,"kind":"podcast_episode"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{materialized}");
    let item = materialized["data"]["items"][0].clone();
    assert_eq!(item["kind"], "podcast_episode");
    assert_eq!(item["source_ref"], reference);
    assert!(!item.to_string().contains(".mp3"));
    let (status, result) = json_request_from(
        router,
        Method::POST,
        "/v1/uni/items/stream",
        Some(json!({"item":item,"quality":"auto","fallback":false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["source_ref"], reference);
    assert_eq!(result["data"]["stream"]["resolved_track"], "kuwo:64403133");
    assert_eq!(result["data"]["extensions"]["transport"], "podcast_audio");
    assert_eq!(result["data"]["extensions"]["episode"]["ref"], reference);
}
