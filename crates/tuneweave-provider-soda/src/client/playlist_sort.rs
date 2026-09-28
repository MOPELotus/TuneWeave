use super::*;
use crate::account::{add_luna_pc_headers, luna_pc_endpoint};
use crate::login::SodaCredential;
use md5::{Digest, Md5};
use reqwest::header::{CONTENT_TYPE, COOKIE};

const PLAYLIST_SORT_PATH: &str = "/luna/me/playlist/media/sort";
const MAX_SORT_MEDIA: usize =
    UPSTREAM_PLAYLIST_PAGE_SIZE as usize * MAX_UPSTREAM_PLAYLIST_PAGES as usize;

impl SodaClient {
    /// Calls the official manual-order endpoint with its PC request envelope and the
    /// exact selected-account session. The provider verifies the full order by readback.
    pub(crate) async fn sort_account_playlist_media(
        &self,
        playlist_id: &str,
        expected_ids: &[String],
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if !canonical_positive_id(playlist_id)
                || expected_ids.is_empty()
                || expected_ids.len() > MAX_SORT_MEDIA
                || expected_ids.iter().any(|id| !canonical_positive_id(id))
            {
                return Err(soda_invalid_request(
                    "Soda playlist ordering requires a complete ordered list of valid track media",
                ));
            }
            if credential.user_id().is_none() {
                return Err(playlist_sort_authentication_required());
            }

            let request = serde_json::json!({
                "playlist_id": playlist_id,
                "media": expected_ids.iter().map(|id| json!({"id": id, "type": "track"})).collect::<Vec<_>>(),
            });
            let body = serde_json::to_vec(&request)
                .map_err(|_| soda_upstream_error("Soda playlist sort request could not be encoded"))?;
            let url = luna_pc_endpoint(PLAYLIST_SORT_PATH, &self.login_device()?)?;
            let body_stub = format!("{:X}", Md5::digest(&body));
            let response = self
                .send_login_request(
                    add_luna_pc_headers(self.login_request(reqwest::Method::POST, url))
                        .header(CONTENT_TYPE, "application/json; charset=utf-8")
                        .header("x-ss-stub", body_stub)
                        .header("x-luna-background-type", "foreground")
                        .header("x-luna-is-background-req", "0")
                        .header("x-luna-is-local-user", "1")
                        .header(COOKIE, credential.cookie_header()?)
                        .body(body),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(playlist_sort_authentication_required());
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
                    "Soda playlist sort returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda playlist sort").await?;
            parse_sort_playlist_ack(&body, playlist_id, expected_ids)?;
            credential.with_response_cookies(&headers)
        }
        .await;
        self.log_upstream_request(
            "playlist_media_sort",
            "api.qishui.com",
            PLAYLIST_SORT_PATH,
            http_status,
            started,
            &result,
        );
        result
    }
}

fn canonical_positive_id(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == value)
}

fn playlist_sort_authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda playlist sorting requires an authenticated account",
    )
    .with_platform(Platform::Soda)
}

fn parse_sort_playlist_ack(body: &[u8], playlist_id: &str, expected_ids: &[String]) -> Result<()> {
    let ack: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist sort returned malformed data"))?;
    let status_code = ack
        .get("status_code")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| {
            ack.get("status_info")
                .and_then(|info| info.get("status_code"))
                .and_then(serde_json::Value::as_i64)
        });
    if matches!(status_code, Some(100_001 | 1_000_016)) {
        return Err(playlist_sort_authentication_required());
    }
    if status_code != Some(0) {
        return Err(soda_upstream_error(
            "Soda playlist sort did not return an explicit successful status",
        ));
    }
    let data = ack.get("data").unwrap_or(&ack);
    let response_id = data
        .get("playlist_id")
        .or_else(|| data.get("playlistId"))
        .and_then(serde_json::Value::as_str);
    if response_id.is_some_and(|id| id != playlist_id) {
        return Err(soda_upstream_error(
            "Soda playlist sort acknowledgement identified a different playlist",
        ));
    }
    if let Some(response_media) = data.get("media") {
        let response_ids = response_media
            .as_array()
            .ok_or_else(|| {
                soda_upstream_error("Soda playlist sort acknowledgement media was malformed")
            })?
            .iter()
            .map(|item| {
                item.get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        soda_upstream_error(
                            "Soda playlist sort acknowledgement media omitted an ID",
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        if response_ids.iter().any(|id| !canonical_positive_id(id))
            || response_ids
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>()
                != expected_ids
        {
            return Err(soda_upstream_error(
                "Soda playlist sort acknowledgement did not match the requested order",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn sort_ack_requires_explicit_success_and_checks_optional_identity_and_order() {
        let expected = ids(&["11", "13", "11"]);
        for success in [
            br#"{"status_code":0}"#.as_slice(),
            br#"{"status_info":{"status_code":0}}"#,
            br#"{"status_code":0,"playlist_id":"42","media":[{"id":"11"},{"id":"13"},{"id":"11"}]}"#,
            br#"{"status_code":0,"data":{"playlistId":"42","media":[{"id":"11"},{"id":"13"},{"id":"11"}]}}"#,
        ] {
            assert!(parse_sort_playlist_ack(success, "42", &expected).is_ok());
        }

        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_code":0,"playlist_id":"7"}"#,
            br#"{"status_code":0,"media":[{"id":"11"},{"id":"11"},{"id":"13"}]}"#,
            br#"{"status_code":0,"media":[{"id":"11"},{"id":"13"}]}"#,
            br#"{"status_code":0,"media":[]}"#,
        ] {
            assert!(parse_sort_playlist_ack(body, "42", &expected).is_err());
        }
        for code in [100_001, 1_000_016] {
            let body = json!({
                "status_code": code,
                "playlist_id": "42",
                "media": [{"id":"11"}, {"id":"13"}, {"id":"11"}],
            });
            assert_eq!(
                parse_sort_playlist_ack(&serde_json::to_vec(&body).unwrap(), "42", &expected)
                    .unwrap_err()
                    .code,
                ErrorCode::AuthenticationRequired
            );
        }
    }

    #[tokio::test]
    async fn sort_wire_uses_official_pc_envelope_and_selected_session_headers() {
        let credential = SodaCredential::test_credential("selected-secret")
            .bind_user("123456")
            .unwrap();
        let expected = ids(&["11", "13", "11"]);
        let body_bytes = serde_json::to_vec(&json!({
            "playlist_id": "42",
            "media": [
                {"id":"11", "type":"track"},
                {"id":"13", "type":"track"},
                {"id":"11", "type":"track"}
            ]
        }))
        .unwrap();
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":0}"#,
            Some("sessionid_ss=rotated; Path=/"),
        )])
        .await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let refreshed = client
            .sort_account_playlist_media("42", &expected, &credential)
            .await
            .unwrap();
        assert!(refreshed.serialize().unwrap().contains("rotated"));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .starts_with("POST /luna/me/playlist/media/sort?aid=386088&app_name=luna_pc")
        );
        assert!(requests[0].contains("user-agent: LunaPC/3.7.0(452316191)"));
        assert!(requests[0].contains("content-type: application/json; charset=utf-8"));
        assert!(requests[0].contains("x-luna-background-type: foreground"));
        assert!(requests[0].contains("x-luna-is-background-req: 0"));
        assert!(requests[0].contains("x-luna-is-local-user: 1"));
        assert!(requests[0].contains("sessionid_ss=selected-secret"));
        assert!(requests[0].contains(&format!("x-ss-stub: {:X}", Md5::digest(&body_bytes))));
        let body = requests[0].split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&body_bytes).unwrap()
        );
    }

    #[tokio::test]
    async fn sort_wire_rejects_missing_account_or_malformed_media_before_network() {
        let (origin, server) = crate::test_http::serve(Vec::new()).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let anonymous = SodaCredential::test_credential("anonymous");
        let error = client
            .sort_account_playlist_media("42", &ids(&["11"]), &anonymous)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);

        let selected = SodaCredential::test_credential("selected")
            .bind_user("123456")
            .unwrap();
        for (playlist_id, expected) in [
            ("0", ids(&["11"])),
            ("42", Vec::new()),
            ("42", ids(&["01"])),
        ] {
            assert!(
                client
                    .sort_account_playlist_media(playlist_id, &expected, &selected)
                    .await
                    .is_err()
            );
        }
        assert!(server.await.unwrap().is_empty());
    }
}
