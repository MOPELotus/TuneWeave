use super::*;
use serde_json::Value;
use tuneweave_core::MiguSongIdentifiers;
const BY_IDS: &str = "https://app.c.nf.migu.cn/resource/song/by-songids/v2.0";
const LOOKUP: &str = "https://app.c.nf.migu.cn/strategy/song-purchase/status/v1.0";
impl MiguClient {
    pub async fn tracks_by_song_ids(&self, ids: &[String]) -> Result<Vec<Track>> {
        validate_ids(ids)?;
        let unique = ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let body = self.song_id_request(BY_IDS, &unique.join("|")).await?;
        parse_tracks(ids, body)
    }
    pub async fn song_identifiers(&self, song_id: &str) -> Result<MiguSongIdentifiers> {
        validate_ids(&[song_id.to_owned()])?;
        let body = self.song_id_request(LOOKUP, song_id).await?;
        let action = body
            .pointer("/data/actionUrl")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if action.len() > 16 * 1024 {
            return Err(migu_upstream_error(
                "Migu identifier action exceeds its limit",
            ));
        }
        // Read only the named identity fields; never follow this application URL.
        let mut hints = BTreeMap::new();
        if let Some((_, query)) = action.split_once('?') {
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                if matches!(key.as_ref(), "songId" | "contentId" | "copyrightId")
                    && (canonical_platform_id(&value).is_none()
                        || hints.insert(key.into_owned(), value.into_owned()).is_some())
                {
                    return Err(migu_upstream_error(
                        "Migu identifier action has invalid or duplicate identity fields",
                    ));
                }
            }
        }
        if hints.get("songId").is_some_and(|id| id != song_id) {
            return Err(migu_upstream_error(
                "Migu identifier action belongs to another song",
            ));
        }
        // Some valid purchase-status actions have no query fields. The complete
        // song-id detail endpoint is the authoritative fallback for those cases.
        let track = self
            .tracks_by_song_ids(&[song_id.to_owned()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| migu_upstream_error("Migu identifier detail is missing"))?;
        let copyright_id = track
            .extensions
            .get("copyright_id")
            .and_then(Value::as_str)
            .filter(|id| canonical_platform_id(id).is_some())
            .ok_or_else(|| migu_upstream_error("Migu identifier detail omitted copyrightId"))?
            .to_owned();
        if hints.get("contentId").is_some_and(|id| *id != track.id)
            || hints
                .get("copyrightId")
                .is_some_and(|id| *id != copyright_id)
        {
            return Err(migu_upstream_error("Migu identifier sources disagree"));
        }
        Ok(MiguSongIdentifiers {
            song_id: song_id.to_owned(),
            content_id: track.id.clone(),
            copyright_id,
            track_ref: track.resource_ref,
        })
    }
    async fn song_id_request(&self, endpoint: &'static str, ids: &str) -> Result<Value> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(self.catalog_endpoint(endpoint)?)
                .header(ACCEPT, "application/json")
                .header("ua", "Android_migu")
                .header("channel", "014X031")
                .query(&[("songId", ids)])
                .send()
                .await
                .map_err(migu_network_error)?;
            status = Some(response.status());
            let bytes = read_bounded_response(response, "Migu song identifier lookup").await?;
            let body: Value = serde_json::from_slice(&bytes).map_err(|json_cause| {
                let mapped = migu_upstream_error("Migu song identifier response is invalid");
                mapped.with_json_source(&json_cause)
            })?;
            if body["code"] != "000000" {
                return Err(migu_upstream_error(
                    "Migu song identifier request was rejected",
                ));
            }
            Ok(body)
        }
        .await;
        self.log_upstream_request(
            "song_identifiers",
            "app.c.nf.migu.cn",
            if endpoint == BY_IDS {
                "/resource/song/by-songids/v2.0"
            } else {
                "/strategy/song-purchase/status/v1.0"
            },
            status,
            started,
            &result,
        );
        result
    }
}
fn validate_ids(ids: &[String]) -> Result<()> {
    if ids.is_empty()
        || ids.len() > 100
        || ids.iter().any(|id| {
            id.is_empty()
                || id.len() > 64
                || !id.bytes().all(|b| b.is_ascii_digit())
                || id.bytes().all(|b| b == b'0')
        })
    {
        return Err(migu_invalid_request(
            "Migu songId lookup requires 1 to 100 positive decimal IDs",
        ));
    }
    Ok(())
}
fn parse_tracks(ids: &[String], body: Value) -> Result<Vec<Track>> {
    let rows = body
        .get("data")
        .and_then(Value::as_array)
        .filter(|v| v.len() <= 100)
        .ok_or_else(|| migu_upstream_error("Migu songId response omitted its track array"))?;
    let expected = ids.iter().collect::<std::collections::BTreeSet<_>>();
    let mut mapped = BTreeMap::new();
    for row in rows {
        let song: MiguSong = serde_json::from_value(row.clone()).map_err(|json_cause| {
            let mapped = migu_upstream_error("Migu songId track is invalid");
            mapped.with_json_source(&json_cause)
        })?;
        if !expected.contains(&song.song_id) || mapped.contains_key(&song.song_id) {
            return Err(migu_upstream_error(
                "Migu songId response contains duplicate or unrelated tracks",
            ));
        }
        let id = song.song_id.clone();
        let track = map_song(song)?;
        mapped.insert(id, track);
    }
    if mapped.len() != expected.len() {
        return Err(TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "Migu songId response omitted requested tracks",
        )
        .with_platform(Platform::Migu));
    }
    ids.iter()
        .map(|id| {
            mapped
                .get(id)
                .cloned()
                .ok_or_else(|| migu_upstream_error("Migu songId result is incomplete"))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn song_id_batches_keep_requested_order_and_duplicates_and_reject_partial_results() {
        let row = |sid: &str, cid: &str| json!({"resourceType":"2","songId":sid,"contentId":cid,"copyrightId":"COPY1","songName":"song"});
        let ids = vec!["2".into(), "1".into(), "2".into()];
        let tracks = parse_tracks(&ids, json!({"data":[row("1","101"),row("2","102")]})).unwrap();
        assert_eq!(
            tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["102", "101", "102"]
        );
        assert!(parse_tracks(&ids, json!({"data":[row("1","101")]})).is_err());
        assert!(parse_tracks(&ids, json!({"data":[row("1","101"),row("1","101")]})).is_err());
        for id in ["", "0", "1|2", " 1", "secret"] {
            assert!(validate_ids(&[id.into()]).is_err());
        }
    }
    #[tokio::test]
    #[ignore = "requires live Migu access"]
    async fn live_song_id_details_and_identifier_lookup() {
        let client = MiguClient::new(&MiguConfig::default()).unwrap();
        let ids = vec!["1114323823".into(), "1114323823".into()];
        let tracks = client.tracks_by_song_ids(&ids).await.unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].id, "600913000002986836");
        let mapping = client.song_identifiers(&ids[0]).await.unwrap();
        assert_eq!(mapping.content_id, tracks[0].id);
        assert!(!mapping.copyright_id.is_empty());
    }
}
