use super::*;
use crate::login::SodaCredential;

pub(crate) struct SodaAccountPlaylistPage {
    pub page: SodaPlaylistPage,
    pub credential: SodaCredential,
}

#[derive(Deserialize)]
struct AccountPlaylistProof {
    status_info: Option<AccountPlaylistStatus>,
    playlist: Option<AccountPlaylistCounts>,
}

#[derive(Deserialize)]
struct AccountPlaylistStatus {
    now: Option<u64>,
}

#[derive(Deserialize)]
struct AccountPlaylistBusinessStatus {
    status_code: Option<i64>,
    status_info: Option<AccountPlaylistBusinessCode>,
}

#[derive(Deserialize)]
struct AccountPlaylistBusinessCode {
    status_code: Option<i64>,
}

#[derive(Deserialize)]
struct AccountPlaylistCounts {
    count_tracks: Option<u64>,
}

impl SodaClient {
    pub(crate) async fn account_playlist_page(
        &self,
        playlist_id: &str,
        cursor: u64,
        count: u32,
        credential: &SodaCredential,
    ) -> Result<SodaAccountPlaylistPage> {
        if credential.user_id().is_none() {
            return Err(playlist_authentication_required());
        }
        let user_id = credential
            .user_id()
            .ok_or_else(playlist_authentication_required)?
            .to_owned();
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = Url::parse(PLAYLIST_DETAIL_ENDPOINT)
                .map_err(|_| soda_upstream_error("Soda playlist endpoint is invalid"))?;
            // The official PC client sends an empty cursor for the first page and
            // uses `count`; retain its request contract for authenticated playlists.
            let upstream_cursor = if cursor == 0 {
                String::new()
            } else {
                cursor.to_string()
            };
            url.query_pairs_mut()
                .append_pair("playlist_id", playlist_id)
                .append_pair("cursor", &upstream_cursor)
                .append_pair("count", &count.to_string())
                .append_pair("aid", SODA_APP_ID)
                .append_pair("app_name", "luna_pc")
                .append_pair("device_platform", "windows")
                .append_pair("channel", "official")
                .append_pair("version_name", "3.5.1")
                .append_pair("version_code", "30050100")
                .append_pair("device_id", &device.device_id)
                .append_pair("iid", &device.install_id);
            let response = self
                .send_login_request(
                    self.login_request(reqwest::Method::GET, url)
                        .header(reqwest::header::ACCEPT, "application/json")
                        .header(reqwest::header::COOKIE, credential.cookie_header()?),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(playlist_authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda account playlist returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda account playlist").await?;
            let page = parse_account_playlist_page(&body, playlist_id, cursor, count, &user_id)?;
            let credential = credential.with_response_cookies(&headers)?;
            Ok(SodaAccountPlaylistPage { page, credential })
        }
        .await;
        self.log_upstream_request(
            "account_playlist_page",
            "api.qishui.com",
            "/luna/pc/playlist/detail",
            http_status,
            started,
            &result,
        );
        result
    }
}

fn playlist_authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda playlist session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

fn parse_account_playlist_page(
    body: &[u8],
    playlist_id: &str,
    cursor: u64,
    count: u32,
    expected_owner_id: &str,
) -> Result<SodaPlaylistPage> {
    if count == 0 || count > UPSTREAM_ACCOUNT_PLAYLIST_PAGE_SIZE {
        return Err(soda_upstream_error(
            "Soda account playlist requested an invalid physical page size",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda account playlist returned malformed data"))?;
    let shape = account_playlist_business_shape(&value);
    let status: AccountPlaylistBusinessStatus =
        serde_json::from_value(value.clone()).map_err(|_| {
            soda_upstream_error(format!(
                "Soda account playlist returned malformed data ({shape})"
            ))
        })?;
    let codes = [
        status.status_code,
        status
            .status_info
            .as_ref()
            .and_then(|info| info.status_code),
    ];
    if codes.contains(&Some(1_000_016)) {
        return Err(playlist_authentication_required());
    }
    if codes.contains(&Some(1_000_005)) {
        return Err(TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "Soda playlist was not found or is not visible to this account",
        )
        .with_platform(Platform::Soda));
    }
    if codes.into_iter().flatten().any(|code| code != 0) {
        return Err(soda_upstream_error(
            "Soda rejected the account playlist request",
        ));
    }
    let proof: AccountPlaylistProof = match serde_json::from_value(value.clone()) {
        Ok(proof) => proof,
        Err(_) => {
            #[cfg(debug_assertions)]
            eprintln!(
                "DIAGNOSTIC soda_account_playlist_proof {}",
                account_playlist_proof_shape(&value)
            );
            return Err(soda_upstream_error(
                "Soda account playlist returned malformed proof",
            ));
        }
    };
    // The official PC playlist screen treats absent media_resources as an empty list and
    // absent has_more as terminal. Accept that shape only for an explicitly identified,
    // zero-count playlist owned by the selected account; never infer an empty library from
    // an incomplete response for someone else's playlist or a later page.
    let metadata_only_empty =
        account_playlist_metadata_only_empty(&value, playlist_id, cursor, count, expected_owner_id);
    let has_complete_page_proof = proof
        .playlist
        .as_ref()
        .and_then(|playlist| playlist.count_tracks)
        .is_some()
        && value
            .get("has_more")
            .and_then(serde_json::Value::as_bool)
            .is_some()
        && value
            .get("media_resources")
            .is_some_and(serde_json::Value::is_array);
    if (!codes.contains(&Some(0))
        && proof
            .status_info
            .as_ref()
            .and_then(|info| info.now)
            .is_none_or(|now| now == 0))
        || (!has_complete_page_proof && !metadata_only_empty)
    {
        #[cfg(debug_assertions)]
        eprintln!(
            "DIAGNOSTIC soda_account_playlist_proof {}",
            account_playlist_proof_shape(&value)
        );
        return Err(soda_upstream_error(
            "Soda account playlist omitted verifiable metadata or pagination",
        ));
    }
    let page = parse_playlist_response(body, playlist_id, cursor, count)?;
    if let Some(count_tracks) = proof
        .playlist
        .as_ref()
        .and_then(|playlist| playlist.count_tracks)
    {
        if count_tracks != page.total {
            return Err(soda_upstream_error(
                "Soda account playlist returned conflicting track counts",
            ));
        }
    } else if !metadata_only_empty || page.total != 0 || !page.tracks.is_empty() || page.has_more {
        return Err(soda_upstream_error(
            "Soda account playlist returned conflicting empty-page metadata",
        ));
    }
    Ok(page)
}

fn account_playlist_metadata_only_empty(
    value: &serde_json::Value,
    expected_playlist_id: &str,
    cursor: u64,
    count: u32,
    expected_owner_id: &str,
) -> bool {
    fn absent_or_zero(value: Option<&serde_json::Value>) -> bool {
        match value {
            None => true,
            Some(value) => value.as_u64() == Some(0),
        }
    }

    if cursor != 0
        || count == 0
        || value.get("media_resources").is_some()
        || value
            .get("has_more")
            .is_some_and(|has_more| has_more.as_bool() != Some(false))
        || value
            .pointer("/status_info/now")
            .and_then(serde_json::Value::as_u64)
            .is_none_or(|now| now == 0)
        || !absent_or_zero(value.pointer("/playlist/count_tracks"))
        || !absent_or_zero(value.pointer("/playlist/resource_cnt/track_cnt"))
        || canonical_positive_decimal(expected_owner_id).is_none()
    {
        return false;
    }

    let playlist_id = value
        .pointer("/playlist/id")
        .and_then(serde_json::Value::as_str);
    let owner_id = value
        .pointer("/playlist/owner/id")
        .and_then(serde_json::Value::as_str);
    playlist_id.is_some_and(|id| canonical_positive_decimal(id) == Some(expected_playlist_id))
        && owner_id == Some(expected_owner_id)
}

fn account_playlist_business_shape(value: &serde_json::Value) -> String {
    fn kind(value: Option<&serde_json::Value>) -> &'static str {
        match value {
            None => "missing",
            Some(serde_json::Value::Null) => "null",
            Some(serde_json::Value::Bool(_)) => "boolean",
            Some(serde_json::Value::Number(_)) => "number",
            Some(serde_json::Value::String(_)) => "string",
            Some(serde_json::Value::Array(_)) => "array",
            Some(serde_json::Value::Object(_)) => "object",
        }
    }

    let info = value.get("status_info");
    format!(
        "root={},status_code={},status_info={},nested_status_code={}",
        kind(Some(value)),
        kind(value.get("status_code")),
        kind(info),
        kind(info.and_then(|info| info.get("status_code"))),
    )
}

fn account_playlist_proof_shape(value: &serde_json::Value) -> String {
    fn kind(value: Option<&serde_json::Value>) -> &'static str {
        match value {
            None => "missing",
            Some(serde_json::Value::Null) => "null",
            Some(serde_json::Value::Bool(_)) => "boolean",
            Some(serde_json::Value::Number(_)) => "number",
            Some(serde_json::Value::String(_)) => "string",
            Some(serde_json::Value::Array(_)) => "array",
            Some(serde_json::Value::Object(_)) => "object",
        }
    }

    fn count_bucket(value: Option<&serde_json::Value>) -> &'static str {
        match value.and_then(serde_json::Value::as_u64) {
            Some(0) => "zero",
            Some(_) => "nonzero",
            None => "unknown",
        }
    }

    let playlist = value.get("playlist");
    let resource_count = playlist.and_then(|playlist| playlist.get("resource_cnt"));
    let stats = playlist.and_then(|playlist| playlist.get("stats"));
    let stat_track_count = [
        "count_tracks",
        "track_count",
        "track_cnt",
        "media_count",
        "media_cnt",
    ]
    .into_iter()
    .map(|key| {
        format!(
            "{key}:{}",
            count_bucket(stats.and_then(|stats| stats.get(key)))
        )
    })
    .collect::<Vec<_>>()
    .join("|");
    format!(
        "root_keys={},has_more={},next_cursor={},status_info={},status_info_keys={},status_info_now={},playlist={},playlist_keys={},count_tracks={},count_tracks_value={},resource_cnt={},resource_cnt_keys={},resource_track_count={},resource_track_count_value={},stats={},stats_keys={},stats_track_count={},media_resources={}",
        safe_shape_keys(Some(value)),
        kind(value.get("has_more")),
        kind(value.get("next_cursor")),
        kind(value.get("status_info")),
        safe_shape_keys(value.get("status_info")),
        kind(value.get("status_info").and_then(|info| info.get("now"))),
        kind(playlist),
        safe_shape_keys(playlist),
        kind(playlist.and_then(|playlist| playlist.get("count_tracks"))),
        count_bucket(playlist.and_then(|playlist| playlist.get("count_tracks"))),
        kind(resource_count),
        safe_shape_keys(resource_count),
        kind(resource_count.and_then(|counts| counts.get("track_cnt"))),
        count_bucket(resource_count.and_then(|counts| counts.get("track_cnt"))),
        kind(stats),
        safe_shape_keys(stats),
        stat_track_count,
        kind(value.get("media_resources")),
    )
}

fn safe_shape_keys(value: Option<&serde_json::Value>) -> String {
    let mut keys = value
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(serde_json::Map::keys)
        .filter(|key| {
            key.len() <= 32
                && key
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .take(20)
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_unstable();
    if keys.is_empty() {
        "none".to_owned()
    } else {
        keys.join("|")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: &serde_json::Value) -> Result<SodaPlaylistPage> {
        parse_account_playlist_page(
            &serde_json::to_vec(value).unwrap(),
            "7200303561195061287",
            0,
            UPSTREAM_ACCOUNT_PLAYLIST_PAGE_SIZE,
            "2186250840705864",
        )
    }

    #[test]
    fn account_playlist_accepts_observed_success_and_explicit_business_success() {
        let fixture = crate::client::test_account_playlist_fixture();
        let page = parse(&fixture).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.raw_total, Some(3));
        assert_eq!(page.tracks[0].resource_ref, page.tracks[1].resource_ref);
        for nested in [false, true] {
            let mut value = fixture.clone();
            value["status_info"] = json!({});
            if nested {
                value["status_info"]["status_code"] = json!(0);
            } else {
                value["status_code"] = json!(0);
            }
            assert!(parse(&value).is_ok());
        }
        let mut empty = fixture;
        empty["playlist"]["count_tracks"] = json!(0);
        empty["media_resources"] = json!([]);
        assert_eq!(parse(&empty).unwrap().tracks.len(), 0);
    }

    #[test]
    fn account_playlist_privacy_preserves_false_and_keeps_missing_or_null_unknown() {
        for (field, expected) in [
            (Some(json!(true)), Some(true)),
            (Some(json!(false)), Some(false)),
            (Some(json!(null)), None),
            (None, None),
        ] {
            let mut value = crate::client::test_account_playlist_fixture();
            match field {
                Some(privacy_value) => value["playlist"]["is_private"] = privacy_value,
                None => {
                    value["playlist"]
                        .as_object_mut()
                        .unwrap()
                        .remove("is_private");
                }
            }
            let page = parse(&value).unwrap();
            assert_eq!(
                page.playlist
                    .extensions
                    .get("is_private")
                    .and_then(serde_json::Value::as_bool),
                expected
            );
        }
    }

    #[test]
    fn account_playlist_requires_explicit_counts_pagination_and_success_evidence() {
        for pointer in [
            "/playlist/count_tracks",
            "/has_more",
            "/media_resources",
            "/status_info/now",
        ] {
            let mut value = crate::client::test_account_playlist_fixture();
            *value.pointer_mut(pointer).unwrap() = serde_json::Value::Null;
            assert!(parse(&value).is_err(), "{pointer}");
        }
        for pointer in ["/playlist/id", "/media_resources/0/id"] {
            let mut value = crate::client::test_account_playlist_fixture();
            *value.pointer_mut(pointer).unwrap() = json!("123");
            assert!(parse(&value).is_err(), "{pointer}");
        }
        let mut value = crate::client::test_account_playlist_fixture();
        value["has_more"] = json!(true);
        for cursor in [json!("0"), json!("10000001"), json!(null)] {
            value["next_cursor"] = cursor;
            assert!(parse(&value).is_err());
        }
        value["next_cursor"] = json!("100");
        value["media_resources"] = json!([]);
        assert!(
            parse(&value).unwrap().has_more,
            "filtered empty pages may continue"
        );
    }

    #[test]
    fn account_playlist_accepts_official_metadata_only_empty_owned_playlist() {
        let value = json!({
            "status_info": {
                "log_id": "private-log-id",
                "now": 1_785_333_200_u64,
                "now_ts_ms": 1_785_333_200_123_u64
            },
            "next_cursor": "",
            "playlist": {
                "id": "7200303561195061287",
                "title": "Empty list",
                "owner": {"id": "2186250840705864"},
                "resource_cnt": {},
                "stats": {},
                "type": 0
            }
        });

        let page = parse(&value).unwrap();
        assert_eq!(page.playlist.id, "7200303561195061287");
        assert_eq!(page.playlist.extensions["owner_id"], "2186250840705864");
        assert_eq!(page.total, 0);
        assert!(page.tracks.is_empty());
        assert!(!page.has_more);
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn account_playlist_metadata_only_empty_shape_requires_owned_zero_count_first_page() {
        let base = json!({
            "status_info": {
                "log_id": "private-log-id",
                "now": 1_785_333_200_u64,
                "now_ts_ms": 1_785_333_200_123_u64
            },
            "playlist": {
                "id": "7200303561195061287",
                "title": "Empty list",
                "owner": {"id": "2186250840705864"},
                "resource_cnt": {},
                "stats": {},
                "type": 0
            }
        });
        let mut cases = Vec::new();

        let mut wrong_owner = base.clone();
        wrong_owner["playlist"]["owner"]["id"] = json!("999");
        cases.push((wrong_owner, 0));

        let mut nonempty_count = base.clone();
        nonempty_count["playlist"]["count_tracks"] = json!(1);
        cases.push((nonempty_count, 0));

        let mut nonempty_raw_count = base.clone();
        nonempty_raw_count["playlist"]["resource_cnt"]["track_cnt"] = json!(1);
        cases.push((nonempty_raw_count, 0));

        let mut resources_present = base.clone();
        resources_present["media_resources"] = json!([]);
        cases.push((resources_present, 0));

        let mut has_more = base.clone();
        has_more["has_more"] = json!(true);
        cases.push((has_more, 0));

        cases.push((base.clone(), 1));

        for (value, cursor) in cases {
            assert!(
                parse_account_playlist_page(
                    &serde_json::to_vec(&value).unwrap(),
                    "7200303561195061287",
                    cursor,
                    UPSTREAM_ACCOUNT_PLAYLIST_PAGE_SIZE,
                    "2186250840705864",
                )
                .is_err()
            );
        }

        assert!(
            parse_account_playlist_page(
                &serde_json::to_vec(&base).unwrap(),
                "7200303561195061287",
                0,
                UPSTREAM_ACCOUNT_PLAYLIST_PAGE_SIZE + 1,
                "2186250840705864",
            )
            .is_err()
        );
    }

    #[test]
    fn account_playlist_classifies_business_failure_before_parsing_payload() {
        for (code, expected) in [
            (1_000_016, ErrorCode::AuthenticationRequired),
            (1_000_005, ErrorCode::ResourceNotFound),
            (7, ErrorCode::UpstreamError),
        ] {
            for nested in [false, true] {
                let mut value =
                    json!({"status_code":0,"status_info":{"status_code":0},"playlist":"malformed"});
                if nested {
                    value["status_info"]["status_code"] = json!(code);
                } else {
                    value["status_code"] = json!(code);
                }
                assert_eq!(parse(&value).err().unwrap().code, expected);
            }
        }
    }
}
