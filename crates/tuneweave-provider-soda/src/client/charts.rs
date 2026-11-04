use super::*;
use tuneweave_core::{Chart, ChartTrackPreview};

pub(crate) const CHART_IDS: [&str; 4] = [
    "7036274230471712007",
    "7060812597884869927",
    "7061475546400005410",
    "7415959718721494311",
];

#[derive(Deserialize)]
struct Envelope {
    status_code: Option<i64>,
    status_info: SodaStatusInfo,
    chart: ChartData,
}
#[derive(Deserialize)]
struct ChartData {
    id: String,
    title: String,
    count_tracks: u64,
    period: String,
    release_time: u64,
    track_ranks: Vec<RankedTrack>,
    style: ChartStyle,
}
#[derive(Deserialize)]
struct ChartStyle {
    desc: String,
}
#[derive(Deserialize)]
struct RankedTrack {
    rank: Rank,
    track: SodaTrack,
}
#[derive(Deserialize)]
struct Rank {
    track_id: String,
    rank_type: String,
    rank_delta: String,
}

impl SodaClient {
    pub(crate) async fn chart_snapshot(&self, id: &str) -> Result<(Chart, Vec<Track>)> {
        if canonical_positive_decimal(id) != Some(id) || id.len() > 64 {
            return Err(soda_invalid_request(
                "Soda chart ID must be a positive decimal ID",
            ));
        }
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let url = Url::parse(&format!("https://api.qishui.com/luna/pc/charts/{id}"))
                .map_err(|_| soda_invalid_request("Invalid Soda chart ID"))?;
            // This public PC endpoint was verified without device state, Cookie,
            // or a signer. Keep account credentials out of the public transport.
            let response = self
                .login_request(reqwest::Method::GET, url)
                .query(&[
                    ("aid", SODA_APP_ID),
                    ("app_name", "luna_pc"),
                    ("device_platform", "windows"),
                    ("channel", "official"),
                ])
                .send()
                .await
                .map_err(soda_network_error)?;
            status = Some(response.status());
            let bytes = read_bounded_response(response, "Soda chart").await?;
            parse_chart(&bytes, id)
        }
        .await;
        self.log_upstream_request(
            "chart",
            "api.qishui.com",
            "/luna/pc/charts/{id}",
            status,
            started,
            &result,
        );
        result
    }
}

fn parse_chart(bytes: &[u8], expected_id: &str) -> Result<(Chart, Vec<Track>)> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|json_cause| {
        let mapped = soda_upstream_error("Soda chart response is invalid");
        mapped.with_json_source(&json_cause)
    })?;
    if envelope.status_code.is_some_and(|code| code != 0) {
        return Err(soda_upstream_error("Soda chart request was rejected"));
    }
    validate_status_metadata(&envelope.status_info, "Soda chart")?;
    let source = envelope.chart;
    if source.id != expected_id
        || canonical_positive_decimal(&source.id).is_none()
        || source.title.trim().is_empty()
        || source.title.len() > 2_000
        || source.period.len() > 64
        || source.style.desc.len() > 64 * 1024
        || source.release_time > 4_102_444_800
        || source.count_tracks > 1_000
        || source.track_ranks.len() as u64 != source.count_tracks
    {
        return Err(soda_upstream_error(
            "Soda chart metadata or complete track count is invalid",
        ));
    }
    let mut tracks = Vec::with_capacity(source.track_ranks.len());
    let mut previews = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, row) in source.track_ranks.into_iter().enumerate() {
        if row.rank.track_id != row.track.id
            || !seen.insert(row.track.id.clone())
            || row.rank.rank_type.len() > 64
            || row.rank.rank_delta.len() > 32
        {
            return Err(soda_upstream_error("Soda chart rank identity is invalid"));
        }
        let mut track = map_track(row.track, "official_pc_chart")?;
        track
            .extensions
            .insert("chart_rank".into(), json!(index + 1));
        // Native rank types and deltas are retained without interpreting their
        // sign or assuming that every rank type has a previous position.
        track
            .extensions
            .insert("rank_type".into(), json!(row.rank.rank_type));
        track
            .extensions
            .insert("rank_delta".into(), json!(row.rank.rank_delta));
        if previews.len() < 3 {
            previews.push(ChartTrackPreview {
                rank: Some((index + 1) as u32),
                previous_rank: None,
                rank_change: None,
                track_ref: Some(track.resource_ref.clone()),
                name: track.name.clone(),
                byline: None,
                cover_url: None,
                extensions: Extensions::new(),
            });
        }
        tracks.push(track);
    }
    let resource_ref = ResourceRef::new(Platform::Soda, format!("chart:{}", source.id))
        .map_err(|_| soda_upstream_error("Soda chart identity is invalid"))?;
    let chart = Chart {
        resource_ref: Some(resource_ref),
        platform: Platform::Soda,
        id: Some(source.id),
        name: source.title.trim().into(),
        description: source.style.desc,
        cover_url: None,
        update_frequency: (!source.period.is_empty()).then_some(source.period),
        updated_at_ms: (source.release_time > 0).then_some(source.release_time * 1_000),
        track_count: Some(source.count_tracks),
        play_count: None,
        subscribed: None,
        playable: None,
        target_kind: Some("chart".into()),
        target_url: None,
        previews,
        extensions: Extensions::from([("backend".into(), json!("official_pc_chart"))]),
    };
    Ok((chart, tracks))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        json!({"status_info":{"log_id":"test","now":1,"now_ts_ms":1000},"chart":{
            "id":"1","title":"chart","count_tracks":1,"period":"weekly","release_time":1,
            "style":{"desc":"description"},"track_ranks":[{"rank":{"track_id":"2","rank_type":"up","rank_delta":"7"},
            "track":{"id":"2","name":"song"}}]}})
    }
    #[test]
    fn chart_mapping_requires_complete_matching_ranked_tracks() {
        let valid = fixture();
        let (chart, tracks) = parse_chart(&serde_json::to_vec(&valid).unwrap(), "1").unwrap();
        assert_eq!(chart.track_count, Some(1));
        assert_eq!(tracks[0].extensions["chart_rank"], 1);
        assert_eq!(tracks[0].extensions["rank_delta"], "7");
        for pointer in [
            "/chart/id",
            "/chart/count_tracks",
            "/chart/track_ranks/0/rank/track_id",
        ] {
            let mut bad = fixture();
            *bad.pointer_mut(pointer).unwrap() = if pointer.ends_with("count_tracks") {
                json!(50)
            } else {
                json!("9")
            };
            assert!(parse_chart(&serde_json::to_vec(&bad).unwrap(), "1").is_err());
        }
        let mut duplicate = fixture();
        let row = duplicate["chart"]["track_ranks"][0].clone();
        duplicate["chart"]["track_ranks"]
            .as_array_mut()
            .unwrap()
            .push(row);
        duplicate["chart"]["count_tracks"] = json!(2);
        assert!(parse_chart(&serde_json::to_vec(&duplicate).unwrap(), "1").is_err());
    }
    #[tokio::test]
    async fn public_chart_transport_uses_fixed_route_without_credentials() {
        let body = fixture().to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (origin, requests) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let provider = crate::SodaProvider::from_client(client);
        use tuneweave_core::{ChartTrackListRequest, MusicProvider};
        let page = provider
            .chart_tracks("chart:1", &ChartTrackListRequest::new(1, 0))
            .await
            .unwrap();
        assert_eq!(page.pagination.total, Some(1));
        assert!(!page.pagination.has_more);
        assert_eq!(page.items[0].id, "2");
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /luna/pc/charts/1?"));
        assert!(requests[0].contains("app_name=luna_pc"));
        assert!(!requests[0].to_ascii_lowercase().contains("cookie:"));
        assert!(!requests[0].contains("device_id="));
        let mut bad = ChartTrackListRequest::new(1, 0);
        bad.account = Some("private".into());
        assert!(provider.chart_tracks("chart:1", &bad).await.is_err());
        bad.account = None;
        bad.limit = 0;
        assert!(provider.chart_tracks("chart:1", &bad).await.is_err());
    }
    #[tokio::test]
    #[ignore = "requires live Soda access"]
    async fn live_public_chart_catalogue_seeds_return_complete_snapshots() {
        let client = SodaClient::new(&SodaConfig::default()).unwrap();
        for id in CHART_IDS {
            let (chart, tracks) = client.chart_snapshot(id).await.unwrap();
            assert_eq!(chart.id.as_deref(), Some(id));
            assert_eq!(chart.track_count, Some(tracks.len() as u64));
            assert!(!tracks.is_empty());
        }
    }
}
