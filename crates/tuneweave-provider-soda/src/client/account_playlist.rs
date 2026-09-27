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
    has_more: Option<bool>,
    media_resources: Option<serde::de::IgnoredAny>,
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
    count_tracks: u64,
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
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = Url::parse(PLAYLIST_DETAIL_ENDPOINT)
                .map_err(|_| soda_upstream_error("Soda playlist endpoint is invalid"))?;
            url.query_pairs_mut()
                .append_pair("playlist_id", playlist_id)
                .append_pair("cursor", &cursor.to_string())
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
            let page = parse_account_playlist_page(&body, playlist_id, cursor, count)?;
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
) -> Result<SodaPlaylistPage> {
    let status: AccountPlaylistBusinessStatus = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda account playlist returned malformed data"))?;
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
    let proof: AccountPlaylistProof = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda account playlist returned malformed data"))?;
    // Official successful PC detail responses can omit status_code; require their actual
    // time, playlist count and explicit pagination payload instead of assuming zero defaults.
    if (!codes.contains(&Some(0))
        && proof
            .status_info
            .as_ref()
            .and_then(|info| info.now)
            .is_none_or(|now| now == 0))
        || proof.has_more.is_none()
        || proof.media_resources.is_none()
        || proof.playlist.is_none()
    {
        return Err(soda_upstream_error(
            "Soda account playlist omitted verifiable metadata or pagination",
        ));
    }
    let page = parse_playlist_response(body, playlist_id, cursor, count)?;
    if proof.playlist.as_ref().unwrap().count_tracks != page.total {
        return Err(soda_upstream_error(
            "Soda account playlist returned conflicting track counts",
        ));
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: &serde_json::Value) -> Result<SodaPlaylistPage> {
        parse_account_playlist_page(
            &serde_json::to_vec(value).unwrap(),
            "7200303561195061287",
            0,
            100,
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
