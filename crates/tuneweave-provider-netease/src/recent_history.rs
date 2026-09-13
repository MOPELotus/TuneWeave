//! Account-owned recent records. These endpoints expose a bounded prefix, not pages.
use super::*;

#[derive(Clone, Copy)]
enum Kind {
    Track,
    Album,
    Playlist,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Self::Track => "/api/play-record/song/list",
            Self::Album => "/api/play-record/album/list",
            Self::Playlist => "/api/play-record/playlist/list",
        }
    }

    fn resource_type(self) -> &'static str {
        match self {
            Self::Track => "SONG",
            Self::Album => "ALBUM",
            Self::Playlist => "PLAYLIST",
        }
    }
}

fn request_payload(request: &PageRequest) -> Result<Value> {
    if !(1..=100).contains(&request.limit) || request.offset != 0 {
        return Err(TuneWeaveError::invalid_request(
            "NetEase recent history requires limit between 1 and 100 and offset zero",
        )
        .with_platform(Platform::Netease));
    }
    Ok(json!({ "limit": request.limit }))
}

async fn fetch(provider: &NeteaseProvider, request: &PageRequest, kind: Kind) -> Result<Value> {
    let payload = request_payload(request)?;
    let account = request.account.as_deref().unwrap_or("default");
    let client = provider.client_for(Some(account))?;
    // Do not issue a request with an anonymous identity or borrow another account.
    if !client.is_authenticated() {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "NetEase recent history requires a logged-in session",
        )
        .with_platform(Platform::Netease));
    }
    let response = client
        .request_weapi(kind.path(), payload)
        .await
        .map_err(public_error)?;
    ensure_account_access(&client, &response.body, "recent history").map_err(public_error)?;
    Ok(response.body)
}

// Upstream text and arbitrary response fields are never returned as error diagnostics.
fn public_error(error: TuneWeaveError) -> TuneWeaveError {
    let mut clean = TuneWeaveError::new(error.code, "NetEase recent history request failed")
        .with_platform(Platform::Netease)
        .retryable(error.retryable);
    if let Some(code) = error.details.get("upstream_code").and_then(Value::as_i64) {
        clean.details = json!({ "upstream_code": code });
    }
    clean
}

fn malformed(field: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase returned invalid recent history {field}"),
    )
    .with_platform(Platform::Netease)
}

fn map_page<T>(
    body: Value,
    request: &PageRequest,
    map: impl Fn(Value) -> Result<T>,
) -> Result<Page<T>> {
    request_payload(request)?;
    ensure_success(&body).map_err(public_error)?;
    let data = body
        .get("data")
        .filter(|v| v.is_object())
        .ok_or_else(|| malformed("data"))?;
    let raw = data
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("list"))?;
    let total = match data.get("total") {
        None | Some(Value::Null) => None,
        Some(value) => Some(json_u64(value).ok_or_else(|| malformed("total"))?),
    };
    // Do not silently drop malformed/deleted items, deduplicate, or reorder records.
    let items = raw
        .iter()
        .take(request.limit as usize)
        .cloned()
        .map(map)
        .collect::<Result<Vec<_>>>()?;
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: request.limit,
            offset: 0,
            total,
            next_offset: None,
            has_more: false,
            extensions: Extensions::from([
                ("continuation_supported".to_owned(), json!(false)),
                ("limit_applied".to_owned(), json!(true)),
            ]),
        },
    })
}

struct Record {
    data: Value,
    played_at: Option<String>,
    device: Option<PlaybackDevice>,
}

fn record(raw: Value, kind: Kind) -> Result<Record> {
    let data = raw
        .get("data")
        .filter(|v| v.is_object())
        .ok_or_else(|| malformed("resource"))?;
    let id = data
        .get("id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or_else(|| malformed("resource id"))?;
    if data
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(|name| name.trim().is_empty())
    {
        return Err(malformed("resource name"));
    }
    if let Some(resource_id) = raw.get("resourceId") {
        if json_u64(resource_id) != Some(id) {
            return Err(malformed("resource identity"));
        }
    }
    if let Some(resource_type) = raw.get("resourceType") {
        if resource_type.as_str() != Some(kind.resource_type()) {
            return Err(malformed("resource type"));
        }
    }
    let played_at = match raw.get("playTime") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let millis = json_u64(value).ok_or_else(|| malformed("play time"))?;
            if millis == 0 {
                None
            } else {
                let date = unix_rfc3339(millis / 1_000).ok_or_else(|| malformed("play time"))?;
                Some(format!(
                    "{}.{:03}Z",
                    date.trim_end_matches('Z'),
                    millis % 1_000
                ))
            }
        }
    };
    let info = raw.get("multiTerminalInfo").filter(|v| v.is_object());
    let operating_system =
        radio_text_field(&raw, &["os"]).or_else(|| info.and_then(|v| radio_text_field(v, &["os"])));
    let name = info.and_then(|v| radio_text_field(v, &["osText", "name"]));
    let icon_url = info.and_then(|v| radio_text_field(v, &["icon", "iconUrl"]));
    let device = (operating_system.is_some() || name.is_some() || icon_url.is_some()).then(|| {
        PlaybackDevice {
            operating_system,
            name,
            icon_url,
            extensions: Extensions::new(),
        }
    });
    Ok(Record {
        data: data.clone(),
        played_at,
        device,
    })
}

fn track(raw: Value) -> Result<RecentTrackHistoryEntry> {
    let record = record(raw, Kind::Track)?;
    let song = serde_json::from_value(record.data).map_err(|_| malformed("track"))?;
    Ok(RecentTrackHistoryEntry {
        track: map_song(song, None)?,
        played_at: record.played_at,
        device: record.device,
        extensions: Extensions::new(),
    })
}

fn album(raw: Value) -> Result<RecentAlbumHistoryEntry> {
    let record = record(raw, Kind::Album)?;
    let album = serde_json::from_value(record.data).map_err(|_| malformed("album"))?;
    Ok(RecentAlbumHistoryEntry {
        album: map_album(album)?,
        played_at: record.played_at,
        device: record.device,
        extensions: Extensions::new(),
    })
}

fn playlist(raw: Value) -> Result<RecentPlaylistHistoryEntry> {
    let record = record(raw, Kind::Playlist)?;
    let known_track_count = record.data.get("trackCount").is_some_and(|v| !v.is_null())
        || record.data.get("trackIds").is_some_and(Value::is_array);
    let playlist = serde_json::from_value(record.data).map_err(|_| malformed("playlist"))?;
    let mut playlist = map_playlist(playlist)?;
    if !known_track_count {
        playlist.track_count = None;
    }
    Ok(RecentPlaylistHistoryEntry {
        playlist,
        played_at: record.played_at,
        device: record.device,
        extensions: Extensions::new(),
    })
}

pub(super) async fn tracks(
    provider: &NeteaseProvider,
    request: &PageRequest,
) -> Result<Page<RecentTrackHistoryEntry>> {
    map_page(fetch(provider, request, Kind::Track).await?, request, track)
}

pub(super) async fn albums(
    provider: &NeteaseProvider,
    request: &PageRequest,
) -> Result<Page<RecentAlbumHistoryEntry>> {
    map_page(fetch(provider, request, Kind::Album).await?, request, album)
}

pub(super) async fn playlists(
    provider: &NeteaseProvider,
    request: &PageRequest,
) -> Result<Page<RecentPlaylistHistoryEntry>> {
    map_page(
        fetch(provider, request, Kind::Playlist).await?,
        request,
        playlist,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn fixture(kind: Kind) -> Value {
        serde_json::from_str(match kind {
            Kind::Track => include_str!("../tests/fixtures/recent-history/song.json"),
            Kind::Album => include_str!("../tests/fixtures/recent-history/album.json"),
            Kind::Playlist => include_str!("../tests/fixtures/recent-history/playlist.json"),
        })
        .unwrap()
    }

    #[test]
    fn recent_history_maps_three_resources_with_milliseconds_and_device() {
        let request = PageRequest::new(100, 0);
        let tracks = map_page(fixture(Kind::Track), &request, track).unwrap();
        let albums = map_page(fixture(Kind::Album), &request, album).unwrap();
        let playlists = map_page(fixture(Kind::Playlist), &request, playlist).unwrap();
        assert_eq!(
            tracks.items[0].track.resource_ref.to_string(),
            "netease:123"
        );
        assert_eq!(tracks.items[0].track.duration_ms, Some(180123));
        assert_eq!(tracks.items[0].track.artists[0].name, "Example artist");
        assert_eq!(
            albums.items[0].album.resource_ref.to_string(),
            "netease:456"
        );
        assert_eq!(albums.items[0].album.track_count, Some(10));
        assert_eq!(
            playlists.items[0].playlist.resource_ref.to_string(),
            "netease:789"
        );
        assert_eq!(
            playlists.items[0].playlist.creator.as_ref().unwrap().name,
            "Example owner"
        );
        for entry in [
            serde_json::to_value(&tracks.items[0]).unwrap(),
            serde_json::to_value(&albums.items[0]).unwrap(),
            serde_json::to_value(&playlists.items[0]).unwrap(),
        ] {
            assert_eq!(entry["played_at"], "2024-01-01T00:00:00.123Z");
            assert_eq!(entry["device"]["operating_system"], "android");
            assert_eq!(entry["device"]["name"], "Android");
            assert_eq!(
                entry["device"]["icon_url"],
                "https://example.test/android.png"
            );
        }
        assert_eq!(tracks.pagination.total, Some(8));
        assert_eq!(tracks.pagination.next_offset, None);
        assert!(!tracks.pagination.has_more);
        assert_eq!(
            tracks.pagination.extensions["continuation_supported"],
            false
        );
    }

    #[test]
    fn recent_history_documented_examples_match_normalized_fixtures() {
        let document = include_str!("../../../docs/recent-history.md");
        let examples: Vec<Value> = document
            .split("```json\n")
            .skip(1)
            .map(|block| serde_json::from_str(block.split("```").next().unwrap()).unwrap())
            .filter(|value: &Value| value.get("played_at").is_some())
            .collect();
        assert_eq!(examples.len(), 3);
        assert_eq!(
            examples[0],
            serde_json::to_value(track(fixture(Kind::Track)["data"]["list"][0].clone()).unwrap())
                .unwrap()
        );
        assert_eq!(
            examples[1],
            serde_json::to_value(album(fixture(Kind::Album)["data"]["list"][0].clone()).unwrap())
                .unwrap()
        );
        assert_eq!(
            examples[2],
            serde_json::to_value(
                playlist(fixture(Kind::Playlist)["data"]["list"][0].clone()).unwrap()
            )
            .unwrap()
        );
    }

    #[test]
    fn recent_history_preserves_duplicates_order_and_unknown_total_without_fake_pages() {
        let mut body = fixture(Kind::Track);
        let mut older = body["data"]["list"][0].clone();
        older["playTime"] = json!(1704067199999_u64);
        body["data"]["list"]
            .as_array_mut()
            .unwrap()
            .extend([older.clone(), older]);
        body["data"].as_object_mut().unwrap().remove("total");
        let page = map_page(body, &PageRequest::new(2, 0), track).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].track.id, page.items[1].track.id);
        assert_eq!(
            page.items[1].played_at.as_deref(),
            Some("2023-12-31T23:59:59.999Z")
        );
        assert_eq!(page.pagination.total, None);
        assert_eq!(page.pagination.next_offset, None);
        assert!(!page.pagination.has_more);
        assert!(
            map_page(
                json!({"code":200,"data":{"list":[]}}),
                &PageRequest::new(1, 0),
                track
            )
            .unwrap()
            .items
            .is_empty()
        );
    }

    #[test]
    fn recent_history_rejects_malformed_resources_and_record_identities() {
        for kind in [Kind::Track, Kind::Album, Kind::Playlist] {
            for (pointer, invalid) in [
                ("/data", Value::Null),
                ("/data/id", json!(0)),
                ("/data/id", json!("123")),
                ("/data/name", json!(" ")),
                ("/resourceId", json!("99999")),
                ("/resourceId", json!({})),
                ("/resourceType", json!("VIDEO")),
                ("/playTime", json!(-1)),
                ("/playTime", json!(true)),
                ("/playTime", json!(1.5)),
                ("/playTime", json!("invalid")),
                ("/playTime", json!(u64::MAX)),
            ] {
                let mut raw = fixture(kind)["data"]["list"][0].clone();
                *raw.pointer_mut(pointer).unwrap() = invalid;
                assert_eq!(
                    record(raw, kind).err().unwrap().code,
                    ErrorCode::UpstreamError,
                    "{pointer}"
                );
            }
        }
        for body in [
            json!({}),
            json!({"code":200}),
            json!({"code":200,"data":[]}),
            json!({"code":200,"data":{"list":null}}),
            json!({"code":200,"data":{"list":[],"total":-1}}),
        ] {
            assert_eq!(
                map_page(body, &PageRequest::new(100, 0), track)
                    .unwrap_err()
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn recent_history_handles_missing_time_and_strips_unrelated_private_fields() {
        let original = fixture(Kind::Track)["data"]["list"][0].clone();
        for time in [Value::Null, json!(0)] {
            let mut raw = original.clone();
            raw["playTime"] = time;
            raw["cookie"] = json!("private-marker");
            raw["multiTerminalInfo"]["token"] = json!("private-marker");
            raw["data"]["Authorization"] = json!("private-marker");
            let result = track(raw).unwrap();
            assert_eq!(result.played_at, None);
            assert!(
                !serde_json::to_string(&result)
                    .unwrap()
                    .contains("private-marker")
            );
        }
        let mut raw = original;
        for key in [
            "playTime",
            "os",
            "multiTerminalInfo",
            "resourceId",
            "resourceType",
        ] {
            raw.as_object_mut().unwrap().remove(key);
        }
        let entry = track(raw).unwrap();
        assert!(entry.played_at.is_none() && entry.device.is_none());
    }

    #[test]
    fn recent_history_request_bounds_and_error_classification_are_explicit() {
        assert_eq!(
            request_payload(&PageRequest::new(100, 0)).unwrap(),
            json!({"limit":100})
        );
        for request in [
            PageRequest::new(0, 0),
            PageRequest::new(101, 0),
            PageRequest::new(u32::MAX, 0),
            PageRequest::new(1, 1),
        ] {
            assert_eq!(
                request_payload(&request).unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        for (code, expected) in [
            (301, ErrorCode::AuthenticationRequired),
            (401, ErrorCode::AuthenticationRequired),
            (403, ErrorCode::PermissionDenied),
            (429, ErrorCode::RateLimited),
            (500, ErrorCode::UpstreamError),
        ] {
            let error = map_page(
                json!({"code":code,"message":"private-marker"}),
                &PageRequest::new(1, 0),
                track,
            )
            .unwrap_err();
            assert_eq!(error.code, expected);
            assert!(!error.to_string().contains("private-marker"));
            assert_eq!(error.details, json!({"upstream_code":code}));
        }
    }

    fn mock_history(
        responses: Vec<Option<&'static str>>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut captures = Vec::new();
            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("mock history timeout: {error}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut raw = Vec::new();
                let (end, length) = loop {
                    let mut block = [0; 4096];
                    let size = socket.read(&mut block).unwrap();
                    assert!(size > 0 && raw.len() < 100_000);
                    raw.extend_from_slice(&block[..size]);
                    if let Some(end) = raw.windows(4).position(|v| v == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                        let length: usize = head
                            .lines()
                            .find_map(|v| v.strip_prefix("content-length:"))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while raw.len() < end + length {
                    let mut block = [0; 4096];
                    let size = socket.read(&mut block).unwrap();
                    assert!(size > 0);
                    raw.extend_from_slice(&block[..size]);
                }
                let captured = String::from_utf8(raw).unwrap();
                let kind = if captured.starts_with("POST /weapi/play-record/song/list ") {
                    Kind::Track
                } else if captured.starts_with("POST /weapi/play-record/album/list ") {
                    Kind::Album
                } else if captured.starts_with("POST /weapi/play-record/playlist/list ") {
                    Kind::Playlist
                } else {
                    panic!("unexpected recent history endpoint")
                };
                assert!(captured.contains("params=") && captured.contains("encSecKey="));
                if response == Some("simulate-timeout") {
                    thread::sleep(Duration::from_millis(300));
                    captures.push(captured);
                    continue;
                }
                let mut body = fixture(kind);
                body["data"]["list"][0]["data"]["name"] =
                    json!(if captured.contains("MUSIC_U=history-a") {
                        "Account A"
                    } else {
                        "Account B"
                    });
                let body = response.map_or_else(|| body.to_string(), str::to_owned);
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                captures.push(captured);
            }
            captures
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn recent_history_encrypts_three_endpoints_and_isolates_concurrent_accounts() {
        let (url, server) = mock_history(vec![None; 4]);
        let provider = NeteaseProvider::new(NeteaseConfig {
            web_base_url: url,
            cookie: Some("MUSIC_U=server-fallback".to_owned()),
            ..Default::default()
        })
        .unwrap();
        let account_a = provider
            .caller_credential_scope(
                &ProviderCredential::new(Platform::Netease, "cookie", "MUSIC_U=history-a", None)
                    .unwrap(),
            )
            .unwrap();
        let account_b = provider
            .caller_credential_scope(
                &ProviderCredential::new(Platform::Netease, "cookie", "MUSIC_U=history-b", None)
                    .unwrap(),
            )
            .unwrap();
        let request = PageRequest::new(100, 0);
        let (a, b) = tokio::join!(tracks(&account_a, &request), tracks(&account_b, &request));
        assert_eq!(a.unwrap().items[0].track.name, "Account A");
        assert_eq!(b.unwrap().items[0].track.name, "Account B");
        assert_eq!(
            albums(&account_a, &request).await.unwrap().items[0]
                .album
                .name,
            "Account A"
        );
        assert_eq!(
            playlists(&account_b, &request).await.unwrap().items[0]
                .playlist
                .name,
            "Account B"
        );
        let captures = server.join().unwrap();
        assert_eq!(
            captures
                .iter()
                .filter(|v| v.contains("MUSIC_U=history-a"))
                .count(),
            2
        );
        assert_eq!(
            captures
                .iter()
                .filter(|v| v.contains("MUSIC_U=history-b"))
                .count(),
            2
        );
        assert!(captures.iter().all(|v| !v.contains("server-fallback")));
        assert!(provider.accounts.read().unwrap().is_empty());
    }

    #[tokio::test]
    async fn recent_history_upstream_failures_and_bad_json_do_not_expose_response_text() {
        let responses = vec![
            Some(r#"{"code":301,"message":"private-marker"}"#),
            Some(r#"{"code":403,"message":"private-marker"}"#),
            Some(r#"{"code":429,"message":"private-marker"}"#),
            Some(r#"{"code":500,"message":"private-marker"}"#),
            Some("private-marker-invalid-json"),
        ];
        let (url, server) = mock_history(responses);
        let provider = NeteaseProvider::new(NeteaseConfig {
            web_base_url: url,
            cookie: Some("MUSIC_U=history-a".to_owned()),
            ..Default::default()
        })
        .unwrap();
        for code in [
            ErrorCode::AuthenticationRequired,
            ErrorCode::PermissionDenied,
            ErrorCode::RateLimited,
            ErrorCode::UpstreamError,
            ErrorCode::UpstreamError,
        ] {
            let error = tracks(&provider, &PageRequest::new(1, 0))
                .await
                .unwrap_err();
            assert_eq!(error.code, code);
            assert!(!format!("{error:?}").contains("private-marker"));
        }
        server.join().unwrap();
    }

    #[tokio::test]
    async fn recent_history_timeout_retains_classification_without_private_diagnostics() {
        let (url, server) = mock_history(vec![Some("simulate-timeout")]);
        let provider = NeteaseProvider::new(NeteaseConfig {
            web_base_url: url,
            cookie: Some("MUSIC_U=history-a".to_owned()),
            timeout: Duration::from_millis(100),
            ..Default::default()
        })
        .unwrap();
        let error = tracks(&provider, &PageRequest::new(1, 0))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamTimeout);
        assert!(error.retryable);
        assert_eq!(error.details, json!({}));
        server.join().unwrap();
    }

    #[test]
    fn recent_history_does_not_invent_unknown_playlist_track_counts() {
        let mut raw = fixture(Kind::Playlist)["data"]["list"][0].clone();
        raw["data"].as_object_mut().unwrap().remove("trackCount");
        assert_eq!(playlist(raw).unwrap().playlist.track_count, None);
    }

    #[tokio::test]
    async fn recent_history_missing_accounts_fail_before_network_and_capabilities_are_independent()
    {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).unwrap();
        let request = PageRequest::new(100, 0);
        assert_eq!(
            tracks(&provider, &request).await.unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            albums(&provider, &request).await.unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            playlists(&provider, &request).await.unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
        for (capability, name) in [
            (Capability::RecentTrackHistory, "recent_track_history"),
            (Capability::RecentAlbumHistory, "recent_album_history"),
            (Capability::RecentPlaylistHistory, "recent_playlist_history"),
        ] {
            assert!(provider.capabilities().contains(&capability));
            assert_eq!(serde_json::to_value(capability).unwrap(), json!(name));
        }
        assert!(
            provider
                .capabilities()
                .contains(&Capability::ListeningHistory)
        );
        assert!(
            provider
                .capabilities()
                .contains(&Capability::RecentPodcastEpisodeHistory)
        );
    }
}
