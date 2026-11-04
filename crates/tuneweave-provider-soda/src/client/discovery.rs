use super::*;

pub(crate) const TAGS: [(&str, &str, &str); 13] = [
    ("0", "每日推荐", "推荐"),
    ("14", "流行", "曲风"),
    ("8", "华语", "语言"),
    ("9", "欧美", "语言"),
    ("20", "国风", "曲风"),
    ("18", "民谣", "曲风"),
    ("15", "摇滚", "曲风"),
    ("38", "说唱", "曲风"),
    ("16", "电子", "曲风"),
    ("19", "R&B", "曲风"),
    ("69", "治愈", "场景"),
    ("45", "睡前", "场景"),
    ("40", "学习", "场景"),
];

#[derive(Deserialize)]
struct Envelope {
    status_code: Option<i64>,
    status_info: SodaStatusInfo,
    has_more: bool,
    inner_block: Vec<Block>,
}
#[derive(Deserialize)]
struct Block {
    resources: Vec<Resource>,
}
#[derive(Deserialize)]
struct Resource {
    #[serde(rename = "type")]
    kind: String,
    resource_id: String,
    entity: serde_json::Value,
}

impl SodaClient {
    pub(crate) async fn discovery_playlists(&self, tag: u32) -> Result<(Vec<Playlist>, bool)> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let url = Url::parse("https://api.qishui.com/luna/pc/discover/mix")
                .map_err(|_| soda_upstream_error("Invalid Soda discovery endpoint"))?;
            let response = self
                .login_request(reqwest::Method::POST, url)
                .query(&[
                    ("aid", SODA_APP_ID),
                    ("app_name", "luna_pc"),
                    ("device_platform", "windows"),
                    ("channel", "official"),
                ])
                .json(
                    &json!({"block_type":"discover_playlist_mix", "feed_discover_extra":{},
                    "latest_douyin_liked_playlist_show_ts":0,"sub_channel_id":tag}),
                )
                .send()
                .await
                .map_err(soda_network_error)?;
            status = Some(response.status());
            let bytes = read_bounded_response(response, "Soda playlist discovery").await?;
            parse_discovery(&bytes)
        }
        .await;
        self.log_upstream_request(
            "playlist_catalog",
            "api.qishui.com",
            "/luna/pc/discover/mix",
            status,
            started,
            &result,
        );
        result
    }
}
fn parse_discovery(bytes: &[u8]) -> Result<(Vec<Playlist>, bool)> {
    let source: Envelope = serde_json::from_slice(bytes).map_err(|json_cause| {
        let mapped = soda_upstream_error("Soda discovery response is invalid");
        mapped.with_json_source(&json_cause)
    })?;
    if source.status_code.is_some_and(|c| c != 0) || source.inner_block.len() > 256 {
        return Err(soda_upstream_error(
            "Soda discovery was rejected or exceeded its limits",
        ));
    }
    validate_status_metadata(&source.status_info, "Soda discovery")?;
    let mut playlists = Vec::new();
    let mut resource_count = 0usize;
    for block in source.inner_block {
        resource_count = resource_count.saturating_add(block.resources.len());
        if resource_count > 1_000 {
            return Err(soda_upstream_error(
                "Soda discovery resource limit exceeded",
            ));
        }
        for resource in block.resources {
            if resource.kind != "playlist" {
                continue;
            }
            let raw = resource
                .entity
                .get("playlist")
                .ok_or_else(|| soda_upstream_error("Soda discovery playlist is missing"))?;
            let count = raw
                .get("count_tracks")
                .map(|v| {
                    v.as_u64()
                        .ok_or_else(|| soda_upstream_error("Soda discovery track count is invalid"))
                })
                .transpose()?;
            let metadata: SodaPlaylistMetadata =
                serde_json::from_value(raw.clone()).map_err(|json_cause| {
                    let mapped = soda_upstream_error("Soda discovery playlist metadata is invalid");
                    mapped.with_json_source(&json_cause)
                })?;
            let (mut playlist, _, _, _) = map_playlist(metadata, &resource.resource_id)?;
            playlist.track_count = count;
            playlist
                .extensions
                .insert("backend".into(), json!("official_pc_discover_mix"));
            if raw.get("stats").is_none() {
                playlist.extensions.remove("stats");
            }
            playlists.push(playlist);
        }
    }
    Ok((playlists, source.has_more))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        json!({"status_info":{"now":1,"now_ts_ms":1000},"has_more":true,"inner_block":[{"resources":[{
            "type":"playlist","resource_id":"1","entity":{"playlist":{"id":"1","title":"playlist"}}}]}]})
    }
    #[test]
    fn discovery_preserves_unknown_count_and_rejects_invalid_tail() {
        let (playlists, more) = parse_discovery(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
        assert!(more);
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].track_count, None);
        let mut bad = fixture();
        bad["inner_block"][0]["resources"].as_array_mut().unwrap().push(json!({"type":"playlist","resource_id":"2","entity":{"playlist":{"id":"3","title":"bad"}}}));
        assert!(parse_discovery(&serde_json::to_vec(&bad).unwrap()).is_err());
        let mut bad = fixture();
        bad["inner_block"][0]["resources"][0]["entity"]["playlist"]["count_tracks"] = json!("bad");
        assert!(parse_discovery(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    #[tokio::test]
    async fn discovery_window_does_not_invent_cursor_pagination_or_sorting() {
        use tuneweave_core::{MusicProvider, PlaylistCatalogKind, PlaylistCatalogRequest};
        let body = fixture().to_string();
        let (origin, requests) =
            crate::test_http::serve(vec![crate::test_http::json(&body, None)]).await;
        let provider = crate::SodaProvider::from_client(
            SodaClient::test_client().with_auth_test_origin(origin),
        );
        let mut request = PlaylistCatalogRequest::new(PlaylistCatalogKind::Tag, 1, 0);
        request.tag_id = Some("0".into());
        let page = provider.playlist_catalog(&request).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.pagination.total, None);
        assert_eq!(page.pagination.next_offset, None);
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["upstream_has_more"], true);
        let requests = requests.await.unwrap();
        assert!(requests[0].starts_with("POST /luna/pc/discover/mix?"));
        assert!(requests[0].contains("\"sub_channel_id\":0"));
        assert!(!requests[0].to_ascii_lowercase().contains("cookie:"));
        request.offset = 1;
        assert!(provider.playlist_catalog(&request).await.is_err());
        request.offset = 0;
        request.catalog = PlaylistCatalogKind::Hot;
        assert!(provider.playlist_catalog(&request).await.is_err());
        request.catalog = PlaylistCatalogKind::Tag;
        request.tag_id = Some("9999".into());
        assert!(provider.playlist_catalog(&request).await.is_err());
    }
    #[tokio::test]
    #[ignore = "requires live Soda access"]
    async fn live_discovery_category_windows() {
        let client = SodaClient::new(&SodaConfig::default()).unwrap();
        for (id, _, _) in TAGS {
            let (rows, _) = client
                .discovery_playlists(id.parse().unwrap())
                .await
                .unwrap();
            assert!(!rows.is_empty(), "empty discovery category {id}");
        }
    }
}
