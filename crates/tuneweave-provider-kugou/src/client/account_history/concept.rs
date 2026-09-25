//! Concept's primary device history is a separate GET protocol and action consumer.
use super::*;
use crate::signing::concept_signature;
use std::time::Duration;

const PATH: &str = "/playhistory/youth/v1/get_songs";
const PAGE_LIMIT: u32 = 64;
const BYTE_LIMIT: usize = 16 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct Envelope {
    status: i64,
    error_code: i64,
    data: Option<Data>,
}
#[derive(Deserialize)]
struct Data {
    userid: WireScalar,
    bp: Option<String>,
    has_more: super::super::dto::Number,
    songs: Vec<WireSong>,
}

#[derive(Default)]
struct Accumulator {
    records: BTreeMap<u64, LatestRecord>,
    cursor: String,
    cursors: BTreeSet<String>,
    pages: u32,
}
impl Accumulator {
    fn accept(&mut self, data: Data, uid: &str) -> Result<bool> {
        if data.userid.into_string() != uid {
            return Err(identity_conflict());
        }
        self.pages += 1;
        if data.has_more.0 > 1 || self.pages > PAGE_LIMIT {
            return Err(malformed());
        }
        for song in data.songs {
            let id = song.mxid.0;
            if id == 0 {
                return Err(malformed());
            }
            if song
                .info
                .as_ref()
                .is_some_and(|info| info.mixsongid.as_ref().map(|v| v.0) != Some(id))
            {
                return Err(identity_conflict());
            }
            let action = action(song.op.0, song.ot.0, song.pc.0)?;
            if let Some(previous) = self.records.get_mut(&id) {
                // pt.e0.w: equal op_time retains the first action. The separate
                // metadata map still receives the last valid info for this mix ID.
                if action.played_at > previous.action.played_at {
                    previous.action = action;
                }
                if song.info.is_some() {
                    previous.info = song.info;
                }
            } else {
                if self.records.len() >= HISTORY_LIMIT {
                    return Err(budget_exceeded());
                }
                self.records.insert(
                    id,
                    LatestRecord {
                        action,
                        info: song.info,
                        device_action_count: 0,
                    },
                );
            }
            // osrs is a Standard action-selection input, not consumed by pt.e0.w.
        }
        if data.has_more.0 == 0 {
            return Ok(true);
        }
        if self.pages == PAGE_LIMIT {
            return Err(budget_exceeded());
        }
        let next = data.bp.ok_or_else(malformed)?;
        if next.is_empty()
            || next.len() > 4096
            || next.chars().any(char::is_control)
            || next == self.cursor
            || !self.cursors.insert(next.clone())
        {
            return Err(malformed());
        }
        self.cursor = next;
        Ok(false)
    }
    fn finish(self) -> Result<AccountHistorySnapshot> {
        let mut items = Vec::new();
        for (id, row) in self.records {
            if row.action.operation == 0 {
                continue;
            }
            items.push(AccountHistoryItem {
                track: map_history_info(id, row.info.ok_or_else(malformed)?)?,
                play_count: row.action.play_count,
                played_at_seconds: row.action.played_at,
                device_action_count: 0,
            });
        }
        items.sort_by(|a, b| {
            b.played_at_seconds
                .cmp(&a.played_at_seconds)
                .then_with(|| a.track.id.cmp(&b.track.id))
        });
        Ok(AccountHistorySnapshot {
            items,
            pages: self.pages,
            complete: true,
        })
    }
}

impl KugouClient {
    pub(crate) async fn concept_account_history(
        &self,
        session: &NativeSession,
        check: impl FnMut() -> Result<()>,
    ) -> Result<AccountHistorySnapshot> {
        if !session.valid() || session.client != KugouLoginClient::Concept {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                tuneweave_core::Capability::ListeningHistory,
            ));
        }
        self.concept_history_with_budget(
            session,
            check,
            tokio::time::Instant::now() + DEADLINE,
            BYTE_LIMIT,
        )
        .await
    }

    async fn concept_history_with_budget(
        &self,
        session: &NativeSession,
        mut check: impl FnMut() -> Result<()>,
        deadline: tokio::time::Instant,
        mut remaining: usize,
    ) -> Result<AccountHistorySnapshot> {
        let mut accumulator = Accumulator::default();
        loop {
            check()?;
            let response = tokio::time::timeout_at(
                deadline,
                self.concept_history_page(
                    session,
                    &accumulator.cursor,
                    remaining.min(HISTORY_RESPONSE_LIMIT),
                ),
            )
            .await;
            check()?;
            let (data, bytes) = response.map_err(|_| budget_exceeded())??;
            if tokio::time::Instant::now() > deadline {
                return Err(budget_exceeded());
            }
            remaining = remaining.checked_sub(bytes).ok_or_else(budget_exceeded)?;
            if accumulator.accept(data, &session.user_id)? {
                return accumulator.finish();
            }
            if remaining == 0 {
                return Err(budget_exceeded());
            }
        }
    }

    async fn concept_history_page(
        &self,
        session: &NativeSession,
        cursor: &str,
        limit: usize,
    ) -> Result<(Data, usize)> {
        let query = parameters(session, cursor, crate::account::now_ms()? / 1000);
        let target = format!("https://gateway.kugou.com{PATH}");
        #[cfg(test)]
        let target = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(PATH).unwrap().to_string())
            .unwrap_or(target);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(target)
                .query(&query)
                .header("accept", "application/json")
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let bytes = crate::account::read_response_with_limit(response, limit).await?;
            let data = parse(&bytes, &session.user_id)?;
            Ok((data, bytes.len()))
        }
        .await;
        self.log_upstream_request(
            "concept_account_history",
            "gateway.kugou.com",
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}
fn parameters(
    session: &NativeSession,
    cursor: &str,
    seconds: u64,
) -> BTreeMap<&'static str, String> {
    let mut query = BTreeMap::from([
        ("userid", session.user_id.clone()),
        ("token", session.token.clone()),
        ("type", "1".into()),
        ("bp", cursor.into()),
        ("appid", session.client.appid().to_string()),
        ("clientver", session.client.clientver().to_string()),
        ("clienttime", seconds.to_string()),
        ("dfid", session.device.dfid().into()),
        ("mid", session.device.mid.clone()),
        ("uuid", "-".into()),
    ]);
    query.insert("signature", concept_signature(&query, b""));
    query
}
fn parse(bytes: &[u8], uid: &str) -> Result<Data> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if matches!(envelope.error_code, 20017 | 20018) {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "KuGou rejected the selected Concept history session",
        )
        .with_platform(Platform::Kugou));
    }
    if envelope.status != 1 || envelope.error_code != 0 {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "KuGou Concept history request was rejected",
        )
        .with_platform(Platform::Kugou)
        .with_details(
            json!({"platform_code":envelope.error_code,"platform_status":envelope.status}),
        ));
    }
    let data = envelope.data.ok_or_else(malformed)?;
    if data.userid.as_uid() != uid {
        return Err(identity_conflict());
    }
    Ok(data)
}
fn budget_exceeded() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "KuGou Concept history exceeded its complete-read budget",
    )
    .with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests;
