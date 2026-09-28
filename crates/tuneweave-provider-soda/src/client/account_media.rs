use crate::login::SodaCredential;
use reqwest::header::ACCEPT;
#[cfg(debug_assertions)]
use serde_json::Value;

use super::*;

const ACCOUNT_TRACK_ENDPOINT: &str = "https://api.qishui.com/luna/pc/track_v2";
const ACCOUNT_TRACK_BACKEND: &str = "official_pc_track_v2";

#[derive(Deserialize)]
struct AccountTrackStatus {
    status_code: Option<i64>,
    status_info: Option<AccountBusinessStatus>,
}

#[derive(Deserialize)]
struct AccountBusinessStatus {
    status_code: Option<i64>,
}

/// Validated metadata from one account-bound response. Player material stays private.
pub(crate) struct SodaAccountTrack {
    pub credential: SodaCredential,
    pub collected: Option<bool>,
    identity: SodaTrackIdentity,
    track: Track,
    body: Vec<u8>,
}

impl SodaAccountTrack {
    pub(crate) async fn playback(
        &self,
        client: &SodaClient,
        requested_bitrate: u64,
    ) -> Result<SodaPlayback> {
        parse_authorized_media(
            client,
            &self.body,
            &self.identity,
            requested_bitrate,
            "Soda account playback",
        )
        .await?
        .deliverable_playback()
    }

    pub(crate) async fn audio_content(
        &self,
        client: &SodaClient,
        requested_bitrate: u64,
    ) -> Result<AudioContent> {
        let media = parse_authorized_media(
            client,
            &self.body,
            &self.identity,
            requested_bitrate,
            "Soda account audio content",
        )
        .await?;
        client.deliver_audio_content(&self.identity, media).await
    }

    pub(crate) fn into_track(self) -> Track {
        self.track
    }

    pub(crate) fn lyrics(&self) -> Result<Lyrics> {
        let mut lyrics = parse_lyrics_response(&self.body, &self.identity)?;
        lyrics
            .extensions
            .insert("backend".to_owned(), json!(ACCOUNT_TRACK_BACKEND));
        Ok(lyrics)
    }

    pub(crate) async fn availability(
        &self,
        client: &SodaClient,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        let mut availability = client
            .availability_body(&self.body, &self.identity, request)
            .await?;
        availability
            .extensions
            .insert("backend".to_owned(), json!(ACCOUNT_TRACK_BACKEND));
        if availability.extensions.get("preview_available") == Some(&json!(true)) {
            availability.message = "Soda only permits a preview for this account".to_owned();
        }
        Ok(availability)
    }
}

impl SodaClient {
    pub(crate) async fn account_track(
        &self,
        identity: &SodaTrackIdentity,
        credential: &SodaCredential,
    ) -> Result<SodaAccountTrack> {
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if credential.user_id().is_none() {
                return Err(account_media_authentication_required());
            }
            let device = self.login_device()?;
            let mut url = Url::parse(ACCOUNT_TRACK_ENDPOINT)
                .map_err(|_| soda_upstream_error("Soda account media endpoint is invalid"))?;
            url.query_pairs_mut()
                .append_pair("aid", "386088")
                .append_pair("app_name", "luna_pc")
                .append_pair("device_id", &device.device_id)
                .append_pair("iid", &device.install_id)
                .append_pair("version_code", "30050100")
                .append_pair("version_name", "3.5.1")
                .append_pair("device_platform", "windows");
            let response = self
                .send_login_request(
                    self.login_request(reqwest::Method::POST, url)
                        .header(ACCEPT, "application/json")
                        .header(reqwest::header::COOKIE, credential.cookie_header()?)
                        .json(&json!({
                            "track_id": identity.id(), "media_type": "track",
                            "queue_type": "search_one_track", "scene_name": "search",
                        })),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(account_media_authentication_required());
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
                    "Soda account media returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda account media").await?;
            let mut snapshot = parse_account_track(body, identity, credential)?;
            snapshot.credential = credential.with_response_cookies(&headers)?;
            Ok(snapshot)
        }
        .await;
        self.log_upstream_request(
            "account_track",
            "api.qishui.com",
            "/luna/pc/track_v2",
            http_status,
            started,
            &result,
        );
        result
    }
}

fn account_media_authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda account media session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

fn parse_account_track(
    body: Vec<u8>,
    identity: &SodaTrackIdentity,
    credential: &SodaCredential,
) -> Result<SodaAccountTrack> {
    let parsed = parse_account_track_fields(&body, identity);
    let (track, collected) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            #[cfg(debug_assertions)]
            eprintln!(
                "DIAGNOSTIC soda_account_track_shape bytes={} shape={}",
                body.len(),
                safe_account_track_shape(&body)
            );
            return Err(error);
        }
    };
    Ok(SodaAccountTrack {
        credential: credential.clone(),
        collected,
        identity: identity.clone(),
        track,
        body,
    })
}

fn parse_account_track_fields(
    body: &[u8],
    identity: &SodaTrackIdentity,
) -> Result<(Track, Option<bool>)> {
    let status: AccountTrackStatus = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda account media returned malformed data"))?;
    let status_info_code = status
        .status_info
        .as_ref()
        .and_then(|info| info.status_code);
    let codes = [status.status_code, status_info_code];
    if codes.contains(&Some(1_000_016)) {
        return Err(account_media_authentication_required());
    }
    if codes.iter().flatten().any(|code| *code != 0) {
        let platform_code = codes
            .into_iter()
            .flatten()
            .find(|code| *code != 0)
            .or(status.status_code)
            .or(status_info_code);
        return Err(
            soda_upstream_error("Soda account media did not report success")
                .with_details(json!({"platform_code":platform_code})),
        );
    }
    // Current track_v2 account responses may omit both status-code members while
    // still returning the account-bound track payload. Treat that as success
    // only when status metadata is present; the exact track identity and the
    // account-bound player payload are validated below before media is exposed.
    if codes.iter().all(Option::is_none) && status.status_info.is_none() {
        return Err(soda_upstream_error(
            "Soda account media omitted status metadata",
        ));
    }
    let envelope: SodaTrackDetailEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda account media returned invalid metadata"))?;
    if envelope.track.is_none() || envelope.seo_track.is_some() {
        return Err(soda_upstream_error(
            "Soda account media omitted its own track payload",
        ));
    }
    let mut track = parse_track_detail_response(body, identity)?;
    track
        .extensions
        .insert("backend".to_owned(), json!(ACCOUNT_TRACK_BACKEND));
    let collected = envelope
        .track
        .as_ref()
        .and_then(|track| track.state.is_collected);
    Ok((track, collected))
}

#[cfg(debug_assertions)]
fn safe_account_track_shape(body: &[u8]) -> Value {
    fn kind(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return json!({"kind":"non_json"});
    };
    let Some(object) = value.as_object() else {
        return json!({"kind":kind(&value)});
    };
    let mut fields = serde_json::Map::new();
    for name in ["status_code", "status_info", "track", "seo_track"] {
        if let Some(value) = object.get(name) {
            fields.insert(name.to_owned(), json!(kind(value)));
        }
    }
    let mut summary = serde_json::Map::new();
    summary.insert("fields".into(), Value::Object(fields));
    if let Some(code) = object.get("status_code").filter(|value| value.is_number()) {
        summary.insert("status_code".into(), code.clone());
    }
    if let Some(code) = object
        .get("status_info")
        .and_then(|info| info.get("status_code"))
        .filter(|value| value.is_number())
    {
        summary.insert("status_info_code".into(), code.clone());
    }
    Value::Object(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn account_metadata_preserves_actual_rights_and_hides_player_material() {
        let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
        let credential = SodaCredential::test_credential("test-secret")
            .bind_user("123456")
            .unwrap();
        for preview in [false, true] {
            let snapshot =
                parse_account_track(test_account_track_fixture(preview), &identity, &credential)
                    .unwrap();
            let availability = snapshot
                .availability(
                    &SodaClient::test_client(),
                    &TrackAvailabilityRequest::new(200_000),
                )
                .await
                .unwrap();
            assert_eq!(availability.playable, !preview);
            assert_eq!(availability.extensions["preview_available"], preview);
            assert_eq!(availability.extensions["backend"], ACCOUNT_TRACK_BACKEND);
            let lyrics = snapshot.lyrics().unwrap();
            assert!(lyrics.plain.as_deref().unwrap().contains("测试"));
            let track = snapshot.into_track();
            assert_eq!(track.id, identity.id());
            assert_eq!(track.extensions["backend"], ACCOUNT_TRACK_BACKEND);
            for serialized in [
                serde_json::to_string(&track).unwrap(),
                serde_json::to_string(&lyrics).unwrap(),
                serde_json::to_string(&availability).unwrap(),
            ] {
                for secret in [
                    "url_player_info",
                    "video_model",
                    "spade_a",
                    "test-secret",
                    "token=private",
                ] {
                    assert!(!serialized.contains(secret));
                }
            }
        }
    }

    #[test]
    fn account_metadata_accepts_account_payload_when_track_v2_omits_status_codes() {
        let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
        let credential = SodaCredential::test_credential("test-secret")
            .bind_user("123456")
            .unwrap();
        let mut body: serde_json::Value =
            serde_json::from_slice(&test_account_track_fixture(false)).unwrap();
        body.as_object_mut().unwrap().remove("status_code");
        body["status_info"]
            .as_object_mut()
            .unwrap()
            .remove("status_code");

        let snapshot =
            parse_account_track(serde_json::to_vec(&body).unwrap(), &identity, &credential)
                .expect("track_v2 account payload without status-code members");

        assert_eq!(snapshot.into_track().id, identity.id());
    }

    #[test]
    fn account_metadata_rejects_missing_status_wrong_identity_and_anonymous_envelopes() {
        let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
        let credential = SodaCredential::test_credential("test-secret")
            .bind_user("123456")
            .unwrap();
        for mutation in 0..8 {
            let mut body: serde_json::Value =
                serde_json::from_slice(&test_account_track_fixture(false)).unwrap();
            match mutation {
                0 => {
                    body.as_object_mut().unwrap().remove("status_code");
                    body.as_object_mut().unwrap().remove("status_info");
                }
                1 => body["status_code"] = json!(1000016),
                2 => body["status_info"]["status_code"] = json!(99),
                3 => body["track"]["id"] = json!("999"),
                4 => body["track"]["media_type"] = json!("video"),
                5 => body["seo_track"] = json!({"track":body["track"].clone()}),
                6 => {
                    body.as_object_mut().unwrap().remove("track");
                }
                _ => body["risk_result"] = json!(1),
            }
            let error = match parse_account_track(
                serde_json::to_vec(&body).unwrap(),
                &identity,
                &credential,
            ) {
                Ok(_) => panic!("invalid account metadata accepted: {mutation}"),
                Err(error) => error,
            };
            assert_eq!(
                error.code,
                if mutation == 1 {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            if mutation == 2 {
                assert_eq!(error.details["platform_code"], 99);
            }
            assert!(!error.to_string().contains("test-secret"));
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn account_status_diagnostics_report_codes_and_shape_without_track_values() {
        let body = br#"{"status_code":9,"status_info":{"status_code":99,"token":"private"},"track":{"title":"private title"}}"#;
        let summary = safe_account_track_shape(body).to_string();
        assert!(summary.contains("\"status_info_code\":99"));
        assert!(summary.contains("\"track\":\"object\""));
        assert!(!summary.contains("private"));
    }
}
