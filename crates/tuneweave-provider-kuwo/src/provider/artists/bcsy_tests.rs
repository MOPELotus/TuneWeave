use super::*;
use crate::client::catalog::tests::{json_response, requests, setup};
use tuneweave_core::{
    ArtistArea, ArtistCatalogRequest, ArtistCategory, ArtistGenre, ErrorCode, MusicProvider,
};

fn catalogue_body() -> serde_json::Value {
    json!({"code":200,"data":{"banner":{},"list":[
        {"id":"3313276","artistName":"中国有声阅读","headPic":"https://img1.kuwo.cn/star/starheads/300/63/55/2848908670.jpg","channelId":"0","priority":"11","status":"1"},
        {"id":"3003400","artistName":"大飞（主播）","headPic":"https://img4.kuwo.cn/star/starheads/120/10/4/4189035932.jpg","channelId":"664","channelName":"郑州音乐广播活力944","priority":"13","status":"1"}
    ]}})
}

#[tokio::test]
async fn baicheng_catalogue_uses_one_anonymous_request_and_returns_only_its_scoped_artists() {
    let mut fixture = setup(vec![json_response(&catalogue_body())]).await;
    let catalog = fixture
        .provider
        .artist_catalog(&ArtistCatalogRequest::new())
        .await
        .unwrap();
    let request = requests(&mut fixture, 1).await.remove(0);
    let request_line = request.lines().next().unwrap();
    assert_eq!(request_line, "GET /api/fmradio/bcsy_list HTTP/1.1");
    assert!(!request.to_ascii_lowercase().contains("cookie:"));
    assert!(!request.to_ascii_lowercase().contains("secret"));
    assert_eq!(catalog.platform, Platform::Kuwo);
    assert_eq!(catalog.artists.len(), 2);
    assert!(catalog.featured_artists.is_empty());
    assert!(catalog.filters.areas.is_empty());
    assert!(catalog.filters.categories.is_empty());
    assert!(catalog.filters.genres.is_empty());
    assert_eq!(
        catalog.extensions["catalog_scope"],
        "baicheng_sound_anchors"
    );
    assert_eq!(catalog.artists[1].resource_ref.to_string(), "kuwo:3003400");
    assert_eq!(
        catalog.artists[1].extensions["radio_channel_ref"],
        "kuwo:fm:664"
    );
}

#[tokio::test]
async fn baicheng_channel_reference_opens_the_existing_radio_detail_identity() {
    let station = json!({
        "code":200,
        "curTime":1790129138586_u64,
        "data":{
            "channel_key":"664",
            "channel_name":"郑州音乐广播活力944",
            "flow_url":"https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_664.m3u8"
        }
    });
    let mut fixture = setup(vec![
        json_response(&catalogue_body()),
        json_response(&station),
    ])
    .await;
    let catalog = fixture
        .provider
        .artist_catalog(&ArtistCatalogRequest::new())
        .await
        .unwrap();
    let station_ref = catalog.artists[1].extensions["radio_channel_ref"]
        .as_str()
        .unwrap();
    let station = fixture
        .provider
        .radio_station(station_ref.strip_prefix("kuwo:").unwrap(), None)
        .await
        .unwrap();

    assert_eq!(station.resource_ref.to_string(), station_ref);
    let requests = requests(&mut fixture, 2).await;
    assert!(requests[0].starts_with("GET /api/fmradio/bcsy_list "));
    assert!(requests[1].starts_with("GET /api/fmradio/radio_info/664 "));
}

#[tokio::test]
async fn baicheng_catalogue_rejects_accounts_and_unverified_filters_before_network() {
    for change in [0, 1, 2, 3] {
        let mut fixture = setup(vec![]).await;
        let mut request = ArtistCatalogRequest::new();
        match change {
            0 => request.account = Some("default".to_owned()),
            1 => request.area = ArtistArea::Chinese,
            2 => request.category = ArtistCategory::Female,
            _ => request.genre = ArtistGenre::Pop,
        }
        let error = fixture.provider.artist_catalog(&request).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(fixture.seen.try_recv().is_err());
    }
}
