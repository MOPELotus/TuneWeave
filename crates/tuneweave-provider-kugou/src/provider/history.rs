use super::*;
use crate::{KugouLoginClient, client::account_history::AccountHistorySnapshot};
use chrono::{SecondsFormat, Utc};
use tuneweave_core::PlaybackHistoryPeriod;

const DEFAULT_ACCOUNT: &str = "default";
const PAGE_LIMIT: u32 = 100;
const WEEK_SECONDS: u64 = 7 * 24 * 60 * 60;

impl KugouProvider {
    pub(super) async fn read_account_history(
        &self,
        request: &PlaybackHistoryRequest,
    ) -> Result<Page<PlaybackHistoryEntry>> {
        validate_request(request)?;
        let account = request.account.as_deref().unwrap_or(DEFAULT_ACCOUNT);
        let unsupported_client = match self.selected(account)? {
            Some((KugouCredential::Native(native), _)) => {
                native.session.client == KugouLoginClient::Web
            }
            Some((KugouCredential::Web(_), _)) => true,
            None => false,
        };
        if unsupported_client {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::ListeningHistory,
            ));
        }
        let mut read = self.begin_native_read(account, None).await?;
        let result = async {
            let session = read.session()?.clone();
            let snapshot = match session.client {
                KugouLoginClient::Standard => {
                    self.client
                        .native_account_history(&session, || self.check_account_read(&mut read))
                        .await?
                }
                KugouLoginClient::Concept => {
                    self.client
                        .concept_account_history(&session, || self.check_account_read(&mut read))
                        .await?
                }
                KugouLoginClient::Web => {
                    return Err(TuneWeaveError::unsupported(
                        Platform::Kugou,
                        Capability::ListeningHistory,
                    ));
                }
            };
            let now = crate::account::now_ms()? / 1000;
            let mut page = map_page(snapshot, request, now)?;
            if session.client == KugouLoginClient::Concept {
                page.pagination.extensions.remove("source_classify");
                page.pagination
                    .extensions
                    .insert("backend".into(), json!("concept_youth_history"));
                page.pagination
                    .extensions
                    .insert("device_type".into(), json!(1));
                page.pagination.extensions.insert(
                    "record_selection_policy".into(),
                    json!("latest op_time; equal time keeps first action; action 0 is deleted"),
                );
                for entry in &mut page.items {
                    entry.extensions.remove("source_classify");
                    entry.extensions.remove("device_action_count");
                    entry.extensions.insert("device_type".into(), json!(1));
                }
            }
            Ok(page)
        }
        .await;
        self.finish_account_read(read, result)
    }
}

fn validate_request(request: &PlaybackHistoryRequest) -> Result<()> {
    if !(1..=PAGE_LIMIT).contains(&request.limit)
        || request.offset.checked_add(request.limit).is_none()
    {
        return Err(kugou_invalid_request(
            "KuGou account history pagination is invalid",
        ));
    }
    Ok(())
}

fn map_page(
    snapshot: AccountHistorySnapshot,
    request: &PlaybackHistoryRequest,
    now: u64,
) -> Result<Page<PlaybackHistoryEntry>> {
    validate_request(request)?;
    let since = match request.period {
        PlaybackHistoryPeriod::AllTime => None,
        PlaybackHistoryPeriod::Week => Some(now.saturating_sub(WEEK_SECONDS)),
    };
    let entries = snapshot
        .items
        .into_iter()
        .filter(|item| since.is_none_or(|since| item.played_at_seconds >= since))
        .map(|item| {
            let timestamp = i64::try_from(item.played_at_seconds)
                .ok()
                .and_then(|seconds| chrono::DateTime::<Utc>::from_timestamp(seconds, 0))
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "KuGou account history returned an invalid playback time",
                    )
                    .with_platform(Platform::Kugou)
                })?
                .to_rfc3339_opts(SecondsFormat::Secs, true);
            Ok(PlaybackHistoryEntry {
                track: item.track,
                play_count: Some(item.play_count),
                score: None,
                last_played_at: Some(timestamp),
                extensions: Extensions::from([
                    ("source_classify".into(), json!("app")),
                    ("source_action".into(), json!(1)),
                    (
                        "device_action_count".into(),
                        json!(item.device_action_count),
                    ),
                ]),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let offset = usize::try_from(request.offset)
        .map_err(|_| kugou_invalid_request("KuGou account history offset is invalid"))?;
    if !snapshot.complete && offset >= entries.len() {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "KuGou history reached its complete-read limit before this offset",
        )
        .with_platform(Platform::Kugou)
        .with_details(json!({"upstream_limit": 1000, "offset": request.offset})));
    }
    let total = snapshot.complete.then_some(entries.len() as u64);
    let items = entries
        .into_iter()
        .skip(offset)
        .take(request.limit as usize)
        .collect::<Vec<_>>();
    let end = request.offset + items.len() as u32;
    let has_more = if snapshot.complete {
        u64::from(end) < total.unwrap_or_default()
    } else {
        true
    };
    let mut extensions = Extensions::from([
        ("source_classify".into(), json!("app")),
        ("complete_read".into(), json!(snapshot.complete)),
        ("deleted_records_filtered".into(), json!(true)),
        ("upstream_pages_fetched".into(), json!(snapshot.pages)),
        ("upstream_record_limit".into(), json!(1000)),
        (
            "record_selection_policy".into(),
            json!("latest op_time, then play_count; action 0 is deleted"),
        ),
    ]);
    if request.period == PlaybackHistoryPeriod::Week {
        extensions.insert("period".into(), json!("week"));
    } else {
        extensions.insert("period".into(), json!("all_time"));
    }
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total,
            next_offset: has_more.then_some(end),
            has_more,
            extensions,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::account_history::AccountHistoryItem;
    use crate::provider::library::tests::store_account;
    use crate::provider::session::tests::{exchange, profile, read, reply, server};
    use tuneweave_core::{MusicProvider, ResourceRef};

    fn item(id: &str, played_at_seconds: u64, play_count: u64) -> AccountHistoryItem {
        AccountHistoryItem {
            track: Track::new(
                ResourceRef::new(Platform::Kugou, id).unwrap(),
                format!("Track {id}"),
            ),
            play_count,
            played_at_seconds,
            device_action_count: 0,
        }
    }

    #[test]
    fn week_window_is_inclusive_at_seven_days_before_local_offset_and_limit() {
        let now = 1_800_000_000;
        let request = PlaybackHistoryRequest {
            period: PlaybackHistoryPeriod::Week,
            limit: 1,
            offset: 1,
            account: None,
        };
        let page = map_page(
            AccountHistorySnapshot {
                items: vec![
                    item("recent", now - 10, 20),
                    item("boundary", now - WEEK_SECONDS, 30),
                    item("old", now - WEEK_SECONDS - 1, 40),
                ],
                pages: 2,
                complete: true,
            },
            &request,
            now,
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].track.id, "boundary");
        assert_eq!(page.items[0].play_count, Some(30));
        assert_eq!(
            page.items[0].last_played_at.as_deref(),
            Some("2027-01-08T08:00:00Z")
        );
        assert_eq!(page.pagination.total, Some(2));
        assert!(!page.pagination.has_more);
    }

    #[test]
    fn incomplete_upstream_read_is_reported_and_never_claimed_as_a_total() {
        let request = PlaybackHistoryRequest {
            period: PlaybackHistoryPeriod::AllTime,
            limit: 2,
            offset: 1,
            account: None,
        };
        let page = map_page(
            AccountHistorySnapshot {
                items: vec![item("a", 20, 3), item("b", 10, 2), item("c", 1, 1)],
                pages: 3,
                complete: false,
            },
            &request,
            30,
        )
        .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.pagination.total, None);
        assert_eq!(page.pagination.next_offset, Some(3));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["deleted_records_filtered"], true);

        let past_limit = PlaybackHistoryRequest {
            offset: 3,
            ..request
        };
        assert_eq!(
            map_page(
                AccountHistorySnapshot {
                    items: vec![item("a", 20, 3), item("b", 10, 2), item("c", 1, 1)],
                    pages: 3,
                    complete: false,
                },
                &past_limit,
                30,
            )
            .unwrap_err()
            .code,
            ErrorCode::UpstreamError
        );
    }

    #[test]
    fn invalid_history_pagination_is_rejected_before_account_io() {
        for request in [
            PlaybackHistoryRequest {
                limit: 0,
                ..PlaybackHistoryRequest::new(PlaybackHistoryPeriod::AllTime, 1, 0)
            },
            PlaybackHistoryRequest {
                limit: PAGE_LIMIT + 1,
                ..PlaybackHistoryRequest::new(PlaybackHistoryPeriod::AllTime, 1, 0)
            },
            PlaybackHistoryRequest {
                offset: u32::MAX,
                limit: 1,
                ..PlaybackHistoryRequest::new(PlaybackHistoryPeriod::AllTime, 1, 0)
            },
        ] {
            assert_eq!(
                validate_request(&request).unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[tokio::test]
    async fn account_history_uses_the_selected_standard_account_and_verified_track_identity() {
        let history = reply(json!({
            "userid":"111",
            "has_more":0,
            "songs":[{
                "mxid":900,
                "op":1,
                "ot":1_800_000_000,
                "pc":7,
                "info":{
                    "mixsongid":900,
                    "name":"Singer - Song",
                    "singername":"Singer",
                    "singerinfo":[{"id":21,"name":"Singer"}],
                    "timelen":180000,
                    "album_id":30,
                    "albuminfo":{"id":30,"name":"Album"}
                }
            }]
        }));
        let mut fixture = server(vec![
            exchange("111", "rotated").into(),
            profile("111").into(),
            history.into(),
        ])
        .await;
        let store = store_account(&mut fixture.provider);
        let other = read(&store, "B");
        let page = fixture
            .provider
            .account_history(&PlaybackHistoryRequest {
                period: PlaybackHistoryPeriod::AllTime,
                limit: 10,
                offset: 0,
                account: Some("A".into()),
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].track.id, "900");
        assert_eq!(page.items[0].track.name, "Song");
        assert_eq!(page.items[0].track.artists[0].name, "Singer");
        assert_eq!(page.items[0].track.album.as_ref().unwrap().name, "Album");
        assert_eq!(page.items[0].play_count, Some(7));
        assert_eq!(page.items[0].score, None);
        assert_eq!(page.items[0].extensions["source_action"], 1);
        assert_eq!(page.pagination.total, Some(1));
        assert!(!page.pagination.has_more);
        assert_eq!(read(&store, "A").native().session.token, "rotated");
        assert_eq!(read(&store, "B"), other);
        assert_eq!(fixture.requests.await.unwrap().len(), 3);
    }
}

#[cfg(test)]
mod concept_tests;
