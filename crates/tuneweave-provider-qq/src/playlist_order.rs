//! Account-scoped, complete-list ordering for QQ's SeqSonglist endpoint.
use std::{collections::BTreeMap, time::Duration};

use serde_json::{Value, json};
use tuneweave_core::{
    ErrorCode, Extensions, Platform, PlaylistTrackOrderRequest, PlaylistTrackOrderResult,
    ResourceRef, Result, TuneWeaveError,
};

use super::{
    QqAccountPlaylistRecord, QqApiRequest, QqBusinessResponse, QqCreatedPlaylistsResponse,
    QqCredential, QqPlaylistDetailOptions, QqPlaylistInfoRecord, QqPlaylistLocator,
    QqPlaylistWriteTrack, QqProvider, QqTrackIdentifier, json_i64, json_u64,
    parse_created_playlists, parse_qq_playlist_detail, parse_qq_playlist_locator,
    parse_qq_strict_track_identifier, qq_authentication_required, qq_created_playlists_request,
    qq_data_error, qq_playlist_detail_request, qq_playlist_wire_song_type, qq_ref,
    validate_created_playlists, validate_qq_media_id, validate_qq_playlist_detail_page,
};

const PAGE_SIZE: u32 = 100;
const MAX_TRACKS: usize = 10_000;
const ORDER_BUDGET: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
struct OrderTrack {
    mid: Option<String>,
    identity: QqPlaylistWriteTrack,
}

struct OrderSession<'a> {
    provider: &'a QqProvider,
    account: &'a str,
    credential: QqCredential,
}

impl QqProvider {
    pub(super) async fn reorder_owned_playlist_tracks(
        &self,
        id: &str,
        request: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        let locator = parse_qq_playlist_locator(id)?;
        let playlist_ref = qq_ref(
            &if locator.playlist_id == 0 {
                format!("dir:{}", locator.directory_id)
            } else {
                locator.playlist_id.to_string()
            },
            "playlist",
        )?;
        let desired = parse_desired(&request.track_refs)?;
        let account = request.account.as_deref().unwrap_or("default");
        let credential = self
            .qq_credential(Some(account))?
            .ok_or_else(|| qq_authentication_required(account, "QQ account was not found"))?;
        if credential.string_music_id().parse::<u64>().ok() != Some(credential.music_id)
            || credential.is_locally_expired()? == Some(true)
        {
            return Err(qq_authentication_required(
                account,
                "QQ playlist ordering requires a valid account credential",
            ));
        }
        let session = OrderSession {
            provider: self,
            account,
            credential,
        };
        let mut dispatched = false;
        let outcome = tokio::time::timeout(ORDER_BUDGET, async {
            let target = session.owned_locator(locator).await?;
            let before = session.read_snapshot(target, desired.len(), false).await?;
            let (changed, ordered) = plan_order(&before, &desired)?;
            if changed {
                session
                    .write_order(target, &ordered, &mut dispatched)
                    .await?;
                let after = session.read_snapshot(target, desired.len(), true).await?;
                if !same_order(&after, &ordered) {
                    return Err(qq_data_error(
                        "QQ complete playlist readback did not confirm the requested track order",
                    ));
                }
            }
            session.ensure_current()?;
            Ok(PlaylistTrackOrderResult {
                playlist_ref,
                track_refs: request.track_refs.clone(),
                snapshot_id: None,
                extensions: Extensions::from([
                    ("backend".to_owned(), json!("qq_seq_songlist")),
                    (
                        "verified_by".to_owned(),
                        json!("complete_playlist_readback"),
                    ),
                    ("no_op".to_owned(), json!(!changed)),
                    (
                        "write_requests_dispatched".to_owned(),
                        json!(u8::from(changed)),
                    ),
                    ("atomic".to_owned(), json!(false)),
                ]),
            })
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "QQ playlist ordering exceeded its time budget",
            )
            .with_platform(Platform::Qq))
        });
        outcome.map_err(|mut error| {
            if dispatched {
                error.retryable = false;
                if !error.details.is_object() {
                    error.details = json!({});
                }
                error.details["write_outcome"] = json!("unconfirmed");
                error.details["write_requests_dispatched"] = json!(1);
            }
            error
        })
    }
}

impl OrderSession<'_> {
    fn ensure_current(&self) -> Result<()> {
        if self.provider.qq_credential(Some(self.account))?.as_ref() != Some(&self.credential)
            || self.credential.is_locally_expired()? == Some(true)
        {
            return Err(qq_authentication_required(
                self.account,
                "QQ account changed or expired during playlist ordering",
            ));
        }
        Ok(())
    }

    async fn read(&self, request: QqApiRequest) -> Result<Value> {
        self.ensure_current()?;
        let response = self
            .provider
            .client
            .request_android_with_credential(&[request], Some(&self.credential))
            .await;
        self.ensure_current()?;
        response?
            .into_iter()
            .next()
            .map(|response| response.data)
            .ok_or_else(|| qq_data_error("QQ playlist ordering request returned no response"))
    }

    async fn owned_locator(&self, locator: QqPlaylistLocator) -> Result<QqPlaylistLocator> {
        let response = self
            .read(qq_created_playlists_request(
                self.credential.string_music_id(),
            ))
            .await?;
        select_owned_locator(
            locator,
            &parse_created_playlists(response)?,
            &self.credential,
        )
    }

    async fn read_snapshot(
        &self,
        target: QqPlaylistLocator,
        expected_len: usize,
        readback: bool,
    ) -> Result<Vec<OrderTrack>> {
        let mut tracks = Vec::new();
        let mut update_time = None;
        let mut offset = 0;
        loop {
            let response = self
                .read(qq_playlist_detail_request(
                    QqPlaylistLocator {
                        directory_id: 0,
                        ..target
                    },
                    QqPlaylistDetailOptions {
                        offset,
                        limit: PAGE_SIZE,
                        only_songs: Some(false),
                        include_tags: false,
                        include_user: true,
                    },
                    None,
                ))
                .await?;
            let page = parse_qq_playlist_detail(response)?;
            validate_qq_playlist_detail_page(&page, offset, PAGE_SIZE)?;
            let info = page.info.as_ref().ok_or_else(|| {
                qq_data_error("QQ playlist ordering cannot establish the owner of a detail page")
            })?;
            check_detail_owner(info, target, &self.credential)?;
            if page.total != expected_len as u64 {
                return Err(if readback {
                    qq_data_error("QQ playlist size changed during ordering")
                } else {
                    complete_order_required()
                });
            }
            if update_time
                .replace(info.update_time)
                .is_some_and(|prior| prior != info.update_time)
            {
                return Err(TuneWeaveError::new(
                    ErrorCode::Conflict,
                    "QQ playlist changed while reading its complete track order",
                )
                .with_platform(Platform::Qq));
            }
            for item in page.songs {
                tracks.push(parse_order_track(&item)?);
            }
            if !page.hasmore {
                return Ok(tracks);
            }
            // Pages must make progress and total_song_num must match the bounded input.
            offset = tracks.len() as u32;
        }
    }

    async fn write_order(
        &self,
        target: QqPlaylistLocator,
        ordered: &[OrderTrack],
        dispatched: &mut bool,
    ) -> Result<()> {
        let response = self
            .provider
            .client
            .request_android_business_checked(
                order_request(target, ordered),
                &[],
                Some(&self.credential),
                || {
                    self.ensure_current()?;
                    *dispatched = true;
                    Ok(())
                },
            )
            .await;
        self.ensure_current()?;
        accept_order_ack(response?)
    }
}

fn select_owned_locator(
    requested: QqPlaylistLocator,
    created: &QqCreatedPlaylistsResponse,
    credential: &QqCredential,
) -> Result<QqPlaylistLocator> {
    validate_created_playlists(created)?;
    let selected = created.playlists.iter().find(|item| {
        if requested.playlist_id != 0 {
            item.id == requested.playlist_id
        } else {
            item.dir_id == requested.directory_id
        }
    });
    let item = selected.ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "QQ playlist ordering requires a playlist in the selected account's created library",
        )
        .with_platform(Platform::Qq)
    })?;
    if created.deleted_ids.contains(&item.id)
        || created
            .playlists
            .iter()
            .filter(|other| other.id == item.id || other.dir_id == item.dir_id)
            .take(2)
            .count()
            > 1
    {
        return Err(qq_data_error(
            "QQ created library returned an ambiguous playlist identity",
        ));
    }
    created_locator(item, credential)
}

fn created_locator(
    item: &QqAccountPlaylistRecord,
    credential: &QqCredential,
) -> Result<QqPlaylistLocator> {
    let uin = item.uin.trim();
    if item.id == 0
        || item.dir_id == 0
        || (!uin.is_empty() && uin != "0" && uin.parse::<u64>().ok() != Some(credential.music_id))
    {
        return Err(qq_data_error(
            "QQ created playlist did not bind its ID and owner directory",
        ));
    }
    Ok(QqPlaylistLocator {
        playlist_id: item.id,
        directory_id: item.dir_id,
    })
}

fn check_detail_owner(
    info: &QqPlaylistInfoRecord,
    target: QqPlaylistLocator,
    credential: &QqCredential,
) -> Result<()> {
    let numeric =
        (info.creator.musicid != 0).then_some(info.creator.musicid == credential.music_id);
    let encrypted = (!info.creator.encrypt_uin.trim().is_empty()
        && !credential.encrypt_uin.trim().is_empty())
    .then_some(info.creator.encrypt_uin.trim() == credential.encrypt_uin.trim());
    if info.id != target.playlist_id
        || (info.dir_id != 0 && info.dir_id != target.directory_id)
        || numeric == Some(false)
        || encrypted == Some(false)
        || (numeric != Some(true) && encrypted != Some(true))
    {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "QQ playlist detail did not match the selected owner and playlist identity",
        )
        .with_platform(Platform::Qq));
    }
    Ok(())
}

fn parse_desired(refs: &[ResourceRef]) -> Result<Vec<QqTrackIdentifier>> {
    if refs.is_empty() || refs.len() > MAX_TRACKS {
        return Err(complete_order_required());
    }
    refs.iter()
        .map(|reference| {
            if reference.platform() != Platform::Qq {
                return Err(complete_order_required());
            }
            parse_qq_strict_track_identifier(reference.id())
        })
        .collect()
}

fn parse_order_track(item: &Value) -> Result<OrderTrack> {
    let song_id = item
        .get("id")
        .and_then(json_u64)
        .filter(|id| *id > 0)
        .ok_or_else(|| qq_data_error("QQ playlist order row is missing a numeric track ID"))?;
    let song_type = item
        .get("type")
        .and_then(json_i64)
        .filter(|kind| *kind >= 0)
        .ok_or_else(|| qq_data_error("QQ playlist order row is missing a valid song type"))?;
    let mid = match item.get("mid") {
        None | Some(Value::Null) => None,
        Some(Value::String(mid)) => {
            let mid = mid.trim();
            (!mid.is_empty()).then(|| mid.to_owned())
        }
        _ => {
            return Err(qq_data_error(
                "QQ playlist order row contains an invalid track MID",
            ));
        }
    };
    if let Some(mid) = &mid {
        validate_qq_media_id(mid, "playlist track MID")
            .map_err(|_| qq_data_error("QQ playlist order row contains an invalid track MID"))?;
    }
    Ok(OrderTrack {
        mid,
        identity: QqPlaylistWriteTrack { song_id, song_type },
    })
}

fn plan_order(
    before: &[OrderTrack],
    desired: &[QqTrackIdentifier],
) -> Result<(bool, Vec<OrderTrack>)> {
    if before.len() != desired.len() {
        return Err(complete_order_required());
    }
    let mut by_id = BTreeMap::<u64, (&OrderTrack, usize)>::new();
    let mut by_mid = BTreeMap::new();
    for track in before {
        let (previous, remaining) = by_id.entry(track.identity.song_id).or_insert((track, 0));
        if previous.identity != track.identity {
            return Err(qq_data_error(
                "QQ playlist has conflicting identities for a numeric track ID",
            ));
        }
        *remaining += 1;
        if let Some(mid) = &track.mid
            && by_mid
                .insert(mid.as_str(), track.identity.song_id)
                .is_some_and(|previous| previous != track.identity.song_id)
        {
            return Err(qq_data_error(
                "QQ playlist has conflicting identities for a track MID",
            ));
        }
    }
    let ordered = desired
        .iter()
        .map(|identity| {
            let id = match identity {
                QqTrackIdentifier::Numeric(id) => *id,
                QqTrackIdentifier::Mid(mid) => *by_mid
                    .get(mid.as_str())
                    .ok_or_else(complete_order_required)?,
            };
            let (track, remaining) = by_id.get_mut(&id).ok_or_else(complete_order_required)?;
            *remaining = remaining
                .checked_sub(1)
                .ok_or_else(complete_order_required)?;
            Ok((*track).clone())
        })
        .collect::<Result<Vec<_>>>()?;
    let changed = !same_order(&ordered, before);
    if changed && by_id.len() != ordered.len() {
        // SeqSonglist has no occurrence ID; upstream has not established that
        // repeating the same pair is lossless. Never risk collapsing duplicates.
        return Err(TuneWeaveError::new(
            ErrorCode::CapabilityNotSupported,
            "QQ playlist ordering cannot safely move duplicate track occurrences",
        )
        .with_platform(Platform::Qq));
    }
    Ok((changed, ordered))
}

fn same_order(left: &[OrderTrack], right: &[OrderTrack]) -> bool {
    // MID helps resolve the request; SeqSonglist uses the numeric song and
    // type pair. Optional display metadata need not be identical on readback.
    left.iter()
        .map(|track| track.identity)
        .eq(right.iter().map(|track| track.identity))
}

fn order_request(target: QqPlaylistLocator, tracks: &[OrderTrack]) -> QqApiRequest {
    QqApiRequest::new(
        "music.musicasset.PlaylistDetailWrite",
        "SeqSonglist",
        json!({
            "dirId": target.directory_id,
            "tid": target.playlist_id,
            "v_songPair": tracks.iter().map(|track| json!({
                "songId": track.identity.song_id,
                "songType": qq_playlist_wire_song_type(track.identity.song_type)
            })).collect::<Vec<_>>()
        }),
    )
}

fn accept_order_ack(response: QqBusinessResponse) -> Result<()> {
    if response.code != 0 {
        return Err(qq_data_error("QQ playlist ordering request was rejected")
            .with_details(json!({ "platform_code": response.code })));
    }
    let ret_code = response
        .data
        .get("retCode")
        .and_then(json_i64)
        .ok_or_else(|| {
            qq_data_error("QQ playlist ordering acknowledgement omitted its result code")
        })?;
    if ret_code != 0 {
        return Err(qq_data_error("QQ playlist ordering was not acknowledged")
            .with_details(json!({ "platform_code": ret_code })));
    }
    Ok(())
}

fn complete_order_required() -> TuneWeaveError {
    TuneWeaveError::invalid_request(
        "QQ playlist ordering requires every current QQ track occurrence exactly once (maximum 10,000)",
    )
    .with_platform(Platform::Qq)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    use tuneweave_core::MusicProvider;

    use super::*;
    use crate::client::{QqClient, QqConfig};

    fn credential() -> QqCredential {
        serde_json::from_value::<QqCredential>(json!({
            "musicid": 123456,
            "musickey": "Q_H_L_loopback-test",
            "encryptUin": "test-owner"
        }))
        .unwrap()
        .normalize()
        .unwrap()
    }

    fn created() -> Value {
        json!({
            "total": 1, "bFinish": true,
            "v_playlist": [{"tid": 12345, "dirId": 101, "uin": "123456"}]
        })
    }

    fn page(ids: &[u64], total: usize, has_more: bool, owner: u64) -> Value {
        json!({
            "dirinfo": {
                "tid": 12345, "dirId": 101, "updateTime": 12,
                "creator": {"musicid": owner}
            },
            "songlist_size": ids.len(),
            "total_song_num": total,
            "hasmore": has_more,
            "songlist": ids.iter().map(|id| json!({
                "id": id, "mid": format!("mid{id}"), "type": if *id == 1 { 1 } else { 113 }
            })).collect::<Vec<_>>()
        })
    }

    struct ScriptedServer {
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<Vec<Value>>>,
    }

    impl ScriptedServer {
        fn launch(replies: Vec<Value>) -> (Self, QqProvider) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let done = Arc::clone(&stop);
            let worker = thread::spawn(move || {
                let mut seen = Vec::new();
                let mut replies = VecDeque::from(replies);
                while !done.load(Ordering::Relaxed) {
                    let mut stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("loopback accept: {error}"),
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut input = BufReader::new(&mut stream);
                    let mut content_length = None;
                    loop {
                        let mut line = String::new();
                        assert_ne!(input.read_line(&mut line).unwrap(), 0);
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            content_length = Some(value.trim().parse::<usize>().unwrap());
                        }
                    }
                    let mut body = vec![0; content_length.unwrap()];
                    input.read_exact(&mut body).unwrap();
                    seen.push(serde_json::from_slice(&body).unwrap());
                    let reply = replies
                        .pop_front()
                        .unwrap_or_else(|| json!({"unexpected": true}));
                    if reply.is_null() {
                        // The server got the request but the caller cannot
                        // tell whether it committed before disconnecting.
                        continue;
                    }
                    let body = json!({"code": 0, "req_0": {"code": 0, "data": reply}}).to_string();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                }
                seen
            });
            let server = Self {
                stop,
                worker: Some(worker),
            };
            let mut client = QqClient::new(QqConfig::default()).unwrap();
            client
                .use_test_api_endpoint(format!("http://{address}"))
                .unwrap();
            let mut provider = QqProvider::from_client(client);
            provider.caller_credential = Some(credential());
            (server, provider)
        }

        fn finish(mut self) -> Vec<Value> {
            self.stop.store(true, Ordering::Relaxed);
            self.worker.take().unwrap().join().unwrap()
        }
    }

    impl Drop for ScriptedServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn request(ids: &[u64]) -> PlaylistTrackOrderRequest {
        PlaylistTrackOrderRequest {
            track_refs: ids
                .iter()
                .map(|id| qq_ref(&id.to_string(), "track").unwrap())
                .collect(),
            account: None,
        }
    }

    #[tokio::test]
    async fn orders_only_after_complete_scoped_paging_and_confirms_full_readback() {
        let ids = (1..=101).collect::<Vec<_>>();
        let mut reordered = ids.clone();
        reordered.swap(0, 100);
        let mut readback = page(&reordered[..100], ids.len(), true, 123456);
        readback["songlist"][0]
            .as_object_mut()
            .unwrap()
            .remove("mid");
        let (server, provider) = ScriptedServer::launch(vec![
            created(),
            page(&ids[..100], ids.len(), true, 123456),
            page(&ids[100..], ids.len(), false, 123456),
            json!({"retCode": 0}),
            readback,
            page(&reordered[100..], ids.len(), false, 123456),
        ]);
        let result = provider
            .reorder_playlist_tracks("dir:101", &request(&reordered))
            .await
            .unwrap();
        assert_eq!(result.track_refs, request(&reordered).track_refs);
        assert_eq!(result.playlist_ref.to_string(), "qq:dir:101");
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        let seen = server.finish();
        assert_eq!(seen.len(), 6);
        assert_eq!(seen[0]["req_0"]["param"]["uin"], "123456");
        for (read_index, offset) in [(1, 0), (2, 100), (4, 0), (5, 100)] {
            assert_eq!(seen[read_index]["req_0"]["method"], "CgiGetDiss");
            assert_eq!(seen[read_index]["req_0"]["param"]["disstid"], 12345);
            assert_eq!(seen[read_index]["req_0"]["param"]["song_begin"], offset);
        }
        let write = &seen[3]["req_0"];
        assert_eq!(write["module"], "music.musicasset.PlaylistDetailWrite");
        assert_eq!(write["method"], "SeqSonglist");
        assert_eq!(write["param"]["dirId"], 101);
        assert_eq!(write["param"]["tid"], 12345);
        let pairs = write["param"]["v_songPair"].as_array().unwrap();
        assert_eq!(pairs.len(), 101);
        assert_eq!(
            pairs.first().unwrap(),
            &json!({"songId": 101, "songType": 113})
        );
        assert_eq!(pairs.last().unwrap(), &json!({"songId": 1, "songType": 0}));
    }

    #[tokio::test]
    async fn wrong_owner_on_a_later_page_and_partial_order_never_send_a_write() {
        let ids = (1..=101).collect::<Vec<_>>();
        let (server, provider) = ScriptedServer::launch(vec![
            created(),
            page(&ids[..100], ids.len(), true, 123456),
            page(&ids[100..], ids.len(), false, 987654),
        ]);
        let error = provider
            .reorder_playlist_tracks("12345", &request(&ids))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert_eq!(server.finish().len(), 3);

        let (server, provider) =
            ScriptedServer::launch(vec![created(), page(&[1, 2], 2, false, 123456)]);
        let error = provider
            .reorder_playlist_tracks("12345", &request(&[1]))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(server.finish().len(), 2);
    }

    #[tokio::test]
    async fn already_ordered_is_read_only_and_bad_ack_or_readback_is_never_retried() {
        let before = page(&[1, 2], 2, false, 123456);
        let (server, provider) = ScriptedServer::launch(vec![created(), before.clone()]);
        let result = provider
            .reorder_playlist_tracks("12345", &request(&[1, 2]))
            .await
            .unwrap();
        assert_eq!(result.extensions["no_op"], true);
        assert_eq!(server.finish().len(), 2);

        for (ack, readback) in [
            (Value::Null, None),
            (json!({}), None),
            (json!({"retCode": 0}), Some(before.clone())),
        ] {
            let mut script = vec![created(), before.clone(), ack];
            let expected_calls = if let Some(page) = readback {
                script.push(page);
                4
            } else {
                3
            };
            let (server, provider) = ScriptedServer::launch(script);
            let error = provider
                .reorder_playlist_tracks("12345", &request(&[2, 1]))
                .await
                .unwrap_err();
            assert!(!error.retryable);
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            let seen = server.finish();
            assert_eq!(seen.len(), expected_calls);
            assert_eq!(
                seen.iter()
                    .filter(|call| call["req_0"]["method"] == "SeqSonglist")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn refuses_ambiguous_identity_and_changed_duplicate_occurrences() {
        let a = parse_order_track(&json!({"id": 1, "type": 1, "mid": "a"})).unwrap();
        let b = parse_order_track(&json!({"id": 2, "type": 113, "mid": "b"})).unwrap();
        let before = [a.clone(), a.clone(), b];
        let refs = |ids: &[&str]| {
            ids.iter()
                .map(|id| qq_ref(id, "track").unwrap())
                .collect::<Vec<_>>()
        };
        let same = parse_desired(&refs(&["a", "1", "b"])).unwrap();
        assert!(!plan_order(&before, &same).unwrap().0);
        let changed = parse_desired(&refs(&["b", "a", "1"])).unwrap();
        assert_eq!(
            plan_order(&before, &changed).unwrap_err().code,
            ErrorCode::CapabilityNotSupported
        );
        let partial = parse_desired(&refs(&["a", "a", "a"])).unwrap();
        assert_eq!(
            plan_order(&before, &partial).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        let mut different_type = a.clone();
        different_type.identity.song_type = 5;
        assert_eq!(
            plan_order(
                &[a, different_type],
                &parse_desired(&refs(&["1", "1"])).unwrap()
            )
            .unwrap_err()
            .code,
            ErrorCode::UpstreamError,
        );
    }
}
