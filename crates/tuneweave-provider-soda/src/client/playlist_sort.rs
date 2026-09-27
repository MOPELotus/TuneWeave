use super::*;
use crate::login::SodaCredential;
use reqwest::header::{ACCEPT, COOKIE};

const PLAYLIST_SORT_PATH: &str = "/luna/me/playlist/media/sort";
const LUNA_API_VERSION: &str = "2023-01-04";
const MAX_SORT_MEDIA: usize =
    UPSTREAM_PLAYLIST_PAGE_SIZE as usize * MAX_UPSTREAM_PLAYLIST_PAGES as usize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SortPlaylistRequest<'a> {
    playlist_id: &'a str,
    media: Vec<SortPlaylistMedia<'a>>,
}

#[derive(Serialize)]
struct SortPlaylistMedia<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    media_type: &'static str,
    attr: SortPlaylistMediaAttr,
}

#[derive(Serialize)]
struct SortPlaylistMediaAttr {
    duration: i32,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SortPlaylistAck {
    status_code: Option<i64>,
    #[serde(rename = "playlistId")]
    playlist_id: Option<String>,
    media: Option<Vec<SortPlaylistAckMedia>>,
}

#[derive(Deserialize)]
struct SortPlaylistAckMedia {
    id: String,
}

impl SodaClient {
    /// Calls the Android manual-order endpoint with the exact selected-account session.
    /// Durations are in the Android NetMedia wire unit: integer milliseconds.
    pub(crate) async fn sort_account_playlist_media(
        &self,
        playlist_id: &str,
        media: &[(String, i32)],
        expected_ids: &[String],
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if !canonical_positive_id(playlist_id)
                || media.is_empty()
                || media.len() > MAX_SORT_MEDIA
                || media
                    .iter()
                    .any(|(id, duration)| !canonical_positive_id(id) || *duration < 0)
                || expected_ids.len() != media.len()
                || expected_ids
                    .iter()
                    .zip(media)
                    .any(|(expected, (actual, _))| expected != actual)
            {
                return Err(soda_invalid_request(
                    "Soda playlist ordering requires a complete ordered list of valid track media",
                ));
            }
            if credential.user_id().is_none() {
                return Err(playlist_sort_authentication_required());
            }

            let request = SortPlaylistRequest {
                playlist_id,
                media: media
                    .iter()
                    .map(|(id, duration)| SortPlaylistMedia {
                        id,
                        media_type: "track",
                        attr: SortPlaylistMediaAttr {
                            duration: *duration,
                        },
                    })
                    .collect(),
            };
            let url = Url::parse(&format!("https://api.qishui.com{PLAYLIST_SORT_PATH}"))
                .map_err(|_| soda_upstream_error("Soda playlist sort endpoint is invalid"))?;
            let response = self
                .send_login_request(
                    self.login_request(reqwest::Method::POST, url)
                        .header(ACCEPT, "application/json")
                        .header("x-luna-api-version", LUNA_API_VERSION)
                        .header("x-luna-is-login", "1")
                        .header(COOKIE, credential.cookie_header()?)
                        .json(&request),
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
    let ack: SortPlaylistAck = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist sort returned malformed data"))?;
    if matches!(ack.status_code, Some(100_001 | 1_000_016)) {
        return Err(playlist_sort_authentication_required());
    }
    if ack.status_code != Some(0) {
        return Err(soda_upstream_error(
            "Soda playlist sort did not return an explicit successful status",
        ));
    }
    let response_id = ack
        .playlist_id
        .as_deref()
        .filter(|id| canonical_positive_id(id));
    let response_media = ack.media.ok_or_else(|| {
        soda_upstream_error("Soda playlist sort omitted the acknowledged media order")
    })?;
    let response_ids = response_media
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    if response_id != Some(playlist_id)
        || response_ids.iter().any(|id| !canonical_positive_id(id))
        || response_ids != expected_ids
    {
        return Err(soda_upstream_error(
            "Soda playlist sort acknowledgement did not match the requested playlist and order",
        ));
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
    fn sort_ack_requires_explicit_success_playlist_identity_and_complete_order() {
        let expected = ids(&["11", "13", "11"]);
        let success =
            br#"{"status_code":0,"playlistId":"42","media":[{"id":"11"},{"id":"13"},{"id":"11"}]}"#;
        assert!(parse_sort_playlist_ack(success, "42", &expected).is_ok());

        for body in [
            br#"{"playlistId":"42","media":[{"id":"11"},{"id":"13"},{"id":"11"}]}"#.as_slice(),
            br#"{"status_code":0,"playlistId":"7","media":[{"id":"11"},{"id":"13"},{"id":"11"}]}"#,
            br#"{"status_code":0,"playlistId":"42","media":[{"id":"11"},{"id":"11"},{"id":"13"}]}"#,
            br#"{"status_code":0,"playlistId":"42","media":[{"id":"11"},{"id":"13"}]}"#,
            br#"{"status_code":0,"playlistId":"42","media":[]}"#,
        ] {
            assert!(parse_sort_playlist_ack(body, "42", &expected).is_err());
        }
        for code in [100_001, 1_000_016] {
            let body = json!({
                "status_code": code,
                "playlistId": "42",
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
    async fn sort_wire_uses_android_route_ordered_net_media_and_selected_session_headers() {
        let credential = SodaCredential::test_credential("selected-secret")
            .bind_user("123456")
            .unwrap();
        let expected = ids(&["11", "13", "11"]);
        let media = vec![
            ("11".to_owned(), 180_822),
            ("13".to_owned(), 201_004),
            ("11".to_owned(), 180_822),
        ];
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":0,"playlistId":"42","media":[{"id":"11"},{"id":"13"},{"id":"11"}]}"#,
            Some("sessionid_ss=rotated; Path=/"),
        )])
        .await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let refreshed = client
            .sort_account_playlist_media("42", &media, &expected, &credential)
            .await
            .unwrap();
        assert!(refreshed.serialize().unwrap().contains("rotated"));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("POST /luna/me/playlist/media/sort HTTP/1.1"));
        assert!(requests[0].contains("x-luna-api-version: 2023-01-04"));
        assert!(requests[0].contains("x-luna-is-login: 1"));
        assert!(requests[0].contains("sessionid_ss=selected-secret"));
        let body = requests[0].split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({
                "playlistId": "42",
                "media": [
                    {"id":"11", "type":"track", "attr":{"duration":180822}},
                    {"id":"13", "type":"track", "attr":{"duration":201004}},
                    {"id":"11", "type":"track", "attr":{"duration":180822}}
                ]
            })
        );
    }

    #[tokio::test]
    async fn sort_wire_rejects_missing_account_or_malformed_media_before_network() {
        let (origin, server) = crate::test_http::serve(Vec::new()).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let anonymous = SodaCredential::test_credential("anonymous");
        let error = client
            .sort_account_playlist_media("42", &[("11".to_owned(), 100)], &ids(&["11"]), &anonymous)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);

        let selected = SodaCredential::test_credential("selected")
            .bind_user("123456")
            .unwrap();
        for (playlist_id, media, expected) in [
            ("0", vec![("11".to_owned(), 100)], ids(&["11"])),
            ("42", Vec::new(), Vec::new()),
            ("42", vec![("11".to_owned(), -1)], ids(&["11"])),
            ("42", vec![("01".to_owned(), 100)], ids(&["01"])),
            ("42", vec![("11".to_owned(), 100)], ids(&["13"])),
        ] {
            assert!(
                client
                    .sort_account_playlist_media(playlist_id, &media, &expected, &selected)
                    .await
                    .is_err()
            );
        }
        assert!(server.await.unwrap().is_empty());
    }
}
