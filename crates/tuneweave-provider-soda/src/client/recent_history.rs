use super::*;
use crate::account::{add_luna_pc_headers, luna_pc_endpoint};
use crate::login::SodaCredential;
use tuneweave_core::RecentTrackHistoryEntry;

const PATH: &str = "/luna/pc/me/recently-played-media";
pub(crate) struct RecentPage {
    pub items: Vec<RecentTrackHistoryEntry>,
    pub total: u64,
    pub next_cursor: Option<String>,
    pub credential: SodaCredential,
}
#[derive(Deserialize)]
struct Envelope {
    status_code: Option<i64>,
    status_info: HistoryStatus,
    media: Vec<Media>,
    total_num: u64,
    next_cursor: Option<String>,
    has_more: Option<bool>,
}
#[derive(Deserialize)]
struct HistoryStatus {
    status_code: Option<i64>,
    #[serde(flatten)]
    metadata: SodaStatusInfo,
}
#[derive(Deserialize)]
struct Media {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    entity: Entity,
    play_start_timestamp: Option<u64>,
}
#[derive(Deserialize)]
struct Entity {
    track: SodaTrack,
}

impl SodaClient {
    pub(crate) async fn recent_history_page(
        &self,
        cursor: &str,
        count: u32,
        credential: &SodaCredential,
    ) -> Result<RecentPage> {
        if credential.user_id().is_none() {
            return Err(history_auth());
        }
        if cursor.len() > 1_024
            || cursor.bytes().any(|b| b.is_ascii_control())
            || !(1..=100).contains(&count)
        {
            return Err(soda_invalid_request(
                "Invalid Soda recent history cursor or count",
            ));
        }
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = luna_pc_endpoint(PATH, &device)?;
            url.query_pairs_mut()
                .append_pair("cursor", cursor)
                .append_pair("count", &count.to_string())
                .append_pair("media_type", "track")
                .append_pair("scene", "");
            let response = self
                .send_login_request(
                    add_luna_pc_headers(self.login_request(reqwest::Method::GET, url))
                        .header(COOKIE, credential.cookie_header()?),
                )
                .await?;
            status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(history_auth());
            }
            let headers = response.headers().clone();
            let bytes = read_bounded_response(response, "Soda recent history").await?;
            let (items, total, next_cursor) = parse_history(&bytes, cursor, count)?;
            Ok(RecentPage {
                items,
                total,
                next_cursor,
                credential: credential.with_response_cookies(&headers)?,
            })
        }
        .await;
        self.log_upstream_request(
            "recent_track_history",
            "api.qishui.com",
            PATH,
            status,
            started,
            &result,
        );
        result
    }
}
fn history_auth() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda recent history requires an authenticated session",
    )
    .with_platform(Platform::Soda)
}
fn parse_history(
    bytes: &[u8],
    cursor: &str,
    count: u32,
) -> Result<(Vec<RecentTrackHistoryEntry>, u64, Option<String>)> {
    // Inspect business status before requiring success-only fields.
    let raw: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| soda_upstream_error("Invalid Soda recent history response"))?;
    for value in [
        raw.get("status_code"),
        raw.pointer("/status_info/status_code"),
    ]
    .into_iter()
    .flatten()
    {
        let code = value
            .as_i64()
            .ok_or_else(|| soda_upstream_error("Invalid Soda recent history status"))?;
        if matches!(code, 100_001 | 1_000_016) {
            return Err(history_auth());
        }
        if code != 0 {
            return Err(soda_upstream_error(
                "Soda recent history request was rejected",
            ));
        }
    }
    let source: Envelope = serde_json::from_value(raw)
        .map_err(|_| soda_upstream_error("Soda recent history omitted required fields"))?;
    if source.status_code.is_some_and(|c| c != 0)
        || source.status_info.status_code.is_some_and(|c| c != 0)
        || source.media.len() > count as usize
        || source.total_num > 1_000_000
        || source.media.len() as u64 > source.total_num
    {
        return Err(soda_upstream_error(
            "Soda recent history counts or status are invalid",
        ));
    }
    validate_status_metadata(&source.status_info.metadata, "Soda recent history")?;
    // The official endpoint omits both fields on terminal pages.
    let next = match (source.has_more, source.next_cursor) {
        (Some(true), Some(next))
            if !next.is_empty()
                && next != cursor
                && next.len() <= 1_024
                && !next.bytes().any(|b| b.is_ascii_control())
                && !source.media.is_empty() =>
        {
            Some(next)
        }
        (Some(true), _) => {
            return Err(soda_upstream_error(
                "Soda recent history cursor did not advance",
            ));
        }
        (None, Some(ref next)) if !next.is_empty() => {
            return Err(soda_upstream_error(
                "Soda recent history omitted continuation status",
            ));
        }
        _ => None,
    };
    let items = source
        .media
        .into_iter()
        .map(|row| {
            if row.kind != "track" || row.id != row.entity.track.id {
                return Err(soda_upstream_error(
                    "Soda recent history returned a mismatched media identity",
                ));
            }
            let mut extensions = Extensions::new();
            if let Some(timestamp) = row.play_start_timestamp {
                if timestamp > 4_102_444_800 {
                    return Err(soda_upstream_error("Invalid Soda recent history timestamp"));
                }
                extensions.insert("play_start_timestamp_seconds".into(), json!(timestamp));
            }
            Ok(RecentTrackHistoryEntry {
                track: map_track(row.entity.track, "official_pc_recently_played")?,
                played_at: row
                    .play_start_timestamp
                    .filter(|t| *t > 0)
                    .and_then(unix_rfc3339),
                device: None,
                extensions,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((items, source.total_num, next))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        json!({"status_info":{"now":1,"now_ts_ms":1000},"media":[{"id":"1","type":"track","entity":{"track":{"id":"1","name":"song"}},"play_start_timestamp":123}],"total_num":2,"has_more":true,"next_cursor":"2"})
    }
    #[test]
    fn recent_history_validates_identity_cursors_and_terminal_counts() {
        let (items, total, next) =
            parse_history(&serde_json::to_vec(&fixture()).unwrap(), "", 2).unwrap();
        assert_eq!(items[0].extensions["play_start_timestamp_seconds"], 123);
        assert_eq!(total, 2);
        assert_eq!(next.as_deref(), Some("2"));
        assert!(parse_history(&serde_json::to_vec(&fixture()).unwrap(), "2", 2).is_err());
        let mut bad = fixture();
        bad["media"][0]["id"] = json!("9");
        assert!(parse_history(&serde_json::to_vec(&bad).unwrap(), "", 2).is_err());
        let mut end = fixture();
        end.as_object_mut().unwrap().remove("has_more");
        end.as_object_mut().unwrap().remove("next_cursor");
        assert_eq!(
            parse_history(&serde_json::to_vec(&end).unwrap(), "", 2)
                .unwrap()
                .2,
            None
        );
        assert_eq!(
            parse_history(br#"{"status_code":1000016}"#, "", 2)
                .err()
                .unwrap()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }
}
