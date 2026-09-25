//! Standard app playback history belongs to the currently selected native account.
use super::*;
use crate::{KugouLoginClient, credential::NativeSession, signing::android_signature};

mod concept;

const HISTORY_PATH: &str = "/playhistory/v1/get_songs";
const HISTORY_LIMIT: usize = 1_000;
const HISTORY_PAGE_LIMIT: usize = 1_000;
const HISTORY_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const MAX_HISTORY_DEVICE_ACTIONS: usize = 128;

#[derive(Debug)]
pub(crate) struct AccountHistorySnapshot {
    pub(crate) items: Vec<AccountHistoryItem>,
    pub(crate) pages: u32,
    pub(crate) complete: bool,
}

#[derive(Debug)]
pub(crate) struct AccountHistoryItem {
    pub(crate) track: Track,
    pub(crate) play_count: u64,
    pub(crate) played_at_seconds: u64,
    pub(crate) device_action_count: u32,
}

#[derive(Serialize)]
struct HistoryRequestBody<'a> {
    userid: u64,
    token: &'a str,
    bp: &'a str,
    source_classify: &'static str,
}

#[derive(Deserialize)]
struct HistoryEnvelope {
    status: i64,
    error_code: i64,
    data: Option<HistoryData>,
}

#[derive(Deserialize)]
struct HistoryData {
    userid: WireScalar,
    bp: Option<WireScalar>,
    has_more: Option<super::dto::Number>,
    #[serde(default)]
    songs: Vec<WireSong>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WireScalar {
    Text(String),
    Number(u64),
}

impl WireScalar {
    fn into_string(self) -> String {
        match self {
            Self::Text(value) => value,
            Self::Number(value) => value.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct WireSong {
    mxid: super::dto::Number,
    op: super::dto::Number,
    ot: super::dto::Number,
    pc: super::dto::Number,
    info: Option<WireInfo>,
    osrs: Option<Value>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct WireInfo {
    mixsongid: Option<super::dto::Number>,
    name: String,
    singername: String,
    singerinfo: Vec<WireSinger>,
    timelen: Option<super::dto::Number>,
    album_id: Option<super::dto::Number>,
    albuminfo: Option<WireAlbum>,
    cover: String,
    mvhash: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct WireSinger {
    id: Option<super::dto::Number>,
    name: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct WireAlbum {
    id: Option<super::dto::Number>,
    name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HistoryAction {
    operation: u8,
    played_at: u64,
    play_count: u64,
}

struct LatestRecord {
    action: HistoryAction,
    info: Option<WireInfo>,
    device_action_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageDecision {
    Continue,
    Finished,
}

struct HistoryAccumulator {
    cursor: String,
    seen_cursors: BTreeSet<String>,
    records: BTreeMap<u64, LatestRecord>,
    pages: u32,
    complete: bool,
    truncated: bool,
}

impl Default for HistoryAccumulator {
    fn default() -> Self {
        Self {
            cursor: String::new(),
            seen_cursors: BTreeSet::from([String::new()]),
            records: BTreeMap::new(),
            pages: 0,
            complete: false,
            truncated: false,
        }
    }
}

impl HistoryAccumulator {
    fn accept(&mut self, data: HistoryData, uid: &str) -> Result<PageDecision> {
        if data.userid.into_string() != uid {
            return Err(identity_conflict());
        }
        self.pages = self.pages.checked_add(1).ok_or_else(malformed)?;
        if self.pages > HISTORY_PAGE_LIMIT as u32 {
            return Err(malformed());
        }

        for song in data.songs {
            if song.mxid.0 == 0 {
                return Err(malformed());
            }
            if !self.records.contains_key(&song.mxid.0) && self.records.len() >= HISTORY_LIMIT {
                self.truncated = true;
                continue;
            }
            merge_song(&mut self.records, song)?;
        }

        let has_more = data.has_more.map(|value| value.0).ok_or_else(malformed)?;
        if has_more > 1 {
            return Err(malformed());
        }
        if self.records.len() >= HISTORY_LIMIT {
            self.complete = has_more == 0 && !self.truncated;
            return Ok(PageDecision::Finished);
        }
        if has_more == 0 {
            self.complete = true;
            return Ok(PageDecision::Finished);
        }
        if self.pages >= HISTORY_PAGE_LIMIT as u32 {
            return Ok(PageDecision::Finished);
        }

        let next = data.bp.map(WireScalar::into_string).ok_or_else(malformed)?;
        if next.is_empty()
            || next.len() > 4096
            || next.chars().any(char::is_control)
            || next == self.cursor
            || !self.seen_cursors.insert(next.clone())
        {
            return Err(malformed());
        }
        self.cursor = next;
        Ok(PageDecision::Continue)
    }

    fn into_snapshot(self) -> Result<AccountHistorySnapshot> {
        let mut items = Vec::new();
        for (mxid, latest) in self.records {
            if latest.action.operation == 0 {
                continue;
            }
            let info = latest.info.ok_or_else(malformed)?;
            let track = map_history_info(mxid, info)?;
            items.push(AccountHistoryItem {
                track,
                play_count: latest.action.play_count,
                played_at_seconds: latest.action.played_at,
                device_action_count: latest.device_action_count,
            });
        }
        items.sort_by(|left, right| {
            right
                .played_at_seconds
                .cmp(&left.played_at_seconds)
                .then_with(|| right.play_count.cmp(&left.play_count))
                .then_with(|| left.track.id.cmp(&right.track.id))
        });
        Ok(AccountHistorySnapshot {
            items,
            pages: self.pages,
            complete: self.complete,
        })
    }
}

impl KugouClient {
    pub(crate) async fn native_account_history(
        &self,
        session: &NativeSession,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<AccountHistorySnapshot> {
        if !session.valid() || session.client != KugouLoginClient::Standard {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                tuneweave_core::Capability::ListeningHistory,
            ));
        }

        let mut accumulator = HistoryAccumulator::default();
        loop {
            check()?;
            let response = self.native_history_page(session, &accumulator.cursor).await;
            check()?;
            let data = response?;
            if accumulator.accept(data, &session.user_id)? == PageDecision::Finished {
                break;
            }
        }
        accumulator.into_snapshot()
    }

    async fn native_history_page(
        &self,
        session: &NativeSession,
        cursor: &str,
    ) -> Result<HistoryData> {
        let seconds = crate::account::now_ms()? / 1000;
        let (parameters, body) = history_request(session, cursor, seconds)?;
        let target = format!("https://gateway.kugou.com{HISTORY_PATH}");
        #[cfg(test)]
        let target = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(HISTORY_PATH).unwrap().to_string())
            .unwrap_or(target);

        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(target)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json")
                .header("user-agent", ANDROID_USER_AGENT)
                .header("dfid", session.device.dfid())
                .header("mid", &session.device.mid)
                .header("clienttime", seconds)
                .header("KG-TID", "27")
                .query(&parameters)
                .body(body)
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let bytes =
                crate::account::read_response_with_limit(response, HISTORY_RESPONSE_LIMIT).await?;
            parse_page(&bytes, &session.user_id)
        }
        .await;
        self.log_upstream_request(
            "native_account_history",
            "gateway.kugou.com",
            HISTORY_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn history_request(
    session: &NativeSession,
    cursor: &str,
    seconds: u64,
) -> Result<(BTreeMap<&'static str, String>, Vec<u8>)> {
    let userid = session.user_id.parse::<u64>().map_err(|_| malformed())?;
    let body = serde_json::to_vec(&HistoryRequestBody {
        userid,
        token: &session.token,
        bp: cursor,
        source_classify: "app",
    })
    .map_err(|_| internal())?;
    let mut parameters = BTreeMap::from([
        ("appid", session.client.appid().to_string()),
        ("clienttime", seconds.to_string()),
        ("clientver", session.client.clientver().to_string()),
        ("dfid", session.device.dfid().to_owned()),
        ("mid", session.device.mid.clone()),
        ("platform", "1".to_owned()),
        // Standard APK's packaged common_query_uuid bit 17 is set; the
        // official ParamGenerator.D(17) then sends the disabled marker.
        ("uuid", "-".to_owned()),
    ]);
    let signature = android_signature(&parameters, &body);
    parameters.insert("signature", signature);
    Ok((parameters, body))
}

fn parse_page(bytes: &[u8], expected_uid: &str) -> Result<HistoryData> {
    let envelope: HistoryEnvelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.error_code == 20017 || envelope.error_code == 20018 {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "KuGou rejected the selected account history session",
        )
        .with_platform(Platform::Kugou));
    }
    if envelope.status != 1 || envelope.error_code != 0 {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "KuGou account history request was rejected",
        )
        .with_platform(Platform::Kugou)
        .with_details(
            json!({"platform_code":envelope.error_code,"platform_status":envelope.status}),
        ));
    }
    let data = envelope.data.ok_or_else(malformed)?;
    // Validate each page before consuming any of its song records.
    if data.userid.as_uid() != expected_uid {
        return Err(identity_conflict());
    }
    Ok(data)
}

impl WireScalar {
    fn as_uid(&self) -> String {
        match self {
            Self::Text(value) => value.clone(),
            Self::Number(value) => value.to_string(),
        }
    }
}

fn merge_song(records: &mut BTreeMap<u64, LatestRecord>, song: WireSong) -> Result<()> {
    let mxid = song.mxid.0;
    if mxid == 0 {
        return Err(malformed());
    }
    if song
        .info
        .as_ref()
        .and_then(|info| info.mixsongid.as_ref())
        .is_some_and(|inner| inner.0 != mxid)
    {
        return Err(identity_conflict());
    }
    let base = action(song.op.0, song.ot.0, song.pc.0)?;
    let device_actions = parse_device_actions(song.osrs.as_ref());
    let action_count = u32::try_from(device_actions.len()).map_err(|_| malformed())?;
    // The Standard consumer combines the base and per-device actions, sorts
    // them by play_count descending, and keeps the first (stable) action.
    let selected = device_actions.into_iter().fold(base, |best, candidate| {
        if candidate.play_count > best.play_count {
            candidate
        } else {
            best
        }
    });
    let previous = records.get(&mxid);
    if previous.is_some_and(|record| !is_newer(selected, record.action)) {
        return Ok(());
    }
    let info = song
        .info
        .or_else(|| previous.and_then(|record| record.info.clone()));
    records.insert(
        mxid,
        LatestRecord {
            action: selected,
            info,
            device_action_count: action_count,
        },
    );
    Ok(())
}

fn action(operation: u64, played_at: u64, play_count: u64) -> Result<HistoryAction> {
    let operation = u8::try_from(operation).map_err(|_| malformed())?;
    if operation > 1 {
        return Err(malformed());
    }
    Ok(HistoryAction {
        operation,
        played_at,
        play_count,
    })
}

fn is_newer(candidate: HistoryAction, previous: HistoryAction) -> bool {
    (candidate.played_at, candidate.play_count) > (previous.played_at, previous.play_count)
}

fn parse_device_actions(value: Option<&Value>) -> Vec<HistoryAction> {
    let Some(Value::Object(actions)) = value else {
        return Vec::new();
    };
    if actions.len() > MAX_HISTORY_DEVICE_ACTIONS {
        return Vec::new();
    }
    let parsed = actions
        .values()
        .map(|value| {
            let object = value.as_object()?;
            action(
                json_u64(object.get("op")?)?,
                json_u64(object.get("ot")?)?,
                json_u64(object.get("pc")?)?,
            )
            .ok()
        })
        .collect::<Option<Vec<_>>>();
    parsed.unwrap_or_default()
}

fn json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(value) => value
            .parse::<u64>()
            .ok()
            .filter(|number| number.to_string() == *value),
        _ => None,
    }
}

fn map_history_info(mxid: u64, info: WireInfo) -> Result<Track> {
    if info.mixsongid.as_ref().map(|value| value.0) != Some(mxid) {
        return Err(identity_conflict());
    }
    let name = safe_text(&info.name, 1024)?.ok_or_else(malformed)?;
    let singer_name = safe_text(&info.singername, 1024)?.unwrap_or_default();
    let resource_ref =
        ResourceRef::new(Platform::Kugou, mxid.to_string()).map_err(|_| malformed())?;
    let mut track = Track::new(
        resource_ref,
        history_title(&name, &singer_name, &info.singerinfo),
    );
    track.artists = history_artists(&info.singerinfo, &singer_name)?;
    track.album = history_album(&info)?;
    track.duration_ms = info
        .timelen
        .as_ref()
        .map(|value| value.0)
        .filter(|value| *value > 0);
    track.mv_ref = safe_text(&info.mvhash, 256)?
        .map(|hash| format!("hash:{hash}"))
        .and_then(|id| ResourceRef::new(Platform::Kugou, id).ok());
    track
        .extensions
        .insert("album_audio_id".into(), json!(mxid.to_string()));
    Ok(track)
}

fn history_title(name: &str, singer_name: &str, singers: &[WireSinger]) -> String {
    if let Some((prefix, title)) = name.split_once(" - ")
        && !title.trim().is_empty()
        && (prefix == singer_name || singers.iter().any(|singer| singer.name == prefix))
    {
        return title.trim().to_owned();
    }
    name.to_owned()
}

fn history_artists(singers: &[WireSinger], fallback: &str) -> Result<Vec<ArtistSummary>> {
    let mut artists = Vec::new();
    for singer in singers {
        let Some(name) = safe_text(&singer.name, 512)? else {
            continue;
        };
        artists.push(ArtistSummary {
            resource_ref: singer
                .id
                .as_ref()
                .filter(|id| id.0 > 0)
                .and_then(|id| ResourceRef::new(Platform::Kugou, id.0.to_string()).ok()),
            name,
        });
    }
    if artists.is_empty()
        && let Some(name) = safe_text(fallback, 1024)?
    {
        artists.push(ArtistSummary {
            resource_ref: None,
            name,
        });
    }
    Ok(artists)
}

fn history_album(info: &WireInfo) -> Result<Option<AlbumSummary>> {
    let inner_id = info
        .albuminfo
        .as_ref()
        .and_then(|album| album.id.as_ref())
        .filter(|id| id.0 > 0)
        .map(|id| id.0);
    let outer_id = info.album_id.as_ref().filter(|id| id.0 > 0).map(|id| id.0);
    if inner_id
        .zip(outer_id)
        .is_some_and(|(left, right)| left != right)
    {
        return Err(identity_conflict());
    }
    let name = info
        .albuminfo
        .as_ref()
        .map(|album| album.name.as_str())
        .map(|value| safe_text(value, 1024))
        .transpose()?
        .flatten();
    let Some(name) = name else {
        return Ok(None);
    };
    let id = inner_id.or(outer_id);
    Ok(Some(AlbumSummary {
        resource_ref: id.and_then(|id| ResourceRef::new(Platform::Kugou, id.to_string()).ok()),
        name,
        cover_url: normalize_image_url(&info.cover),
    }))
}

fn safe_text(value: &str, limit: usize) -> Result<Option<String>> {
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(malformed());
    }
    let value = value.trim();
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

fn identity_conflict() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "KuGou account history returned a mismatched owner or track identity",
    )
    .with_platform(Platform::Kugou)
}

fn malformed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "KuGou account history response is invalid",
    )
    .with_platform(Platform::Kugou)
}

fn internal() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "KuGou history request could not be encoded",
    )
    .with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests;
