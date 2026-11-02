use super::*;
use crate::account::{add_luna_pc_headers, luna_pc_endpoint};
use crate::login::SodaCredential;

impl SodaClient {
    pub(crate) async fn account_music_wall(
        &self,
        user: Option<&str>,
        credential: &SodaCredential,
    ) -> Result<(serde_json::Value, SodaCredential)> {
        let owner = credential.user_id().ok_or_else(wall_auth)?;
        let expected_user = user.unwrap_or(owner);
        if canonical_positive_decimal(expected_user) != Some(expected_user)
            || expected_user.len() > 64
        {
            return Err(soda_invalid_request(
                "Soda music wall requires a canonical user ID",
            ));
        }
        let path = user
            .map(|id| format!("/luna/users/{id}/music_wall"))
            .unwrap_or_else(|| "/luna/me/music_wall".into());
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = luna_pc_endpoint(&path, &device)?;
            let builder = if user.is_some() {
                url.query_pairs_mut().append_pair("ab_param", "");
                self.login_request(reqwest::Method::GET, url)
            } else {
                self.login_request(reqwest::Method::POST, url)
                    .json(&json!({"ab_param":"","recent_rankings_version":"","schema_extra":""}))
            };
            let response = self
                .send_login_request(
                    add_luna_pc_headers(builder).header(COOKIE, credential.cookie_header()?),
                )
                .await?;
            status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(wall_auth());
            }
            let headers = response.headers().clone();
            let bytes = read_bounded_response(response, "Soda music wall").await?;
            let wall = parse_wall(&bytes, expected_user, user.is_some())?;
            Ok((wall, credential.with_response_cookies(&headers)?))
        }
        .await;
        self.log_upstream_request(
            "music_wall",
            "api.qishui.com",
            if user.is_some() {
                "/luna/users/{user_id}/music_wall"
            } else {
                "/luna/me/music_wall"
            },
            status,
            started,
            &result,
        );
        result
    }
}
fn wall_auth() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda music wall requires an authenticated session",
    )
    .with_platform(Platform::Soda)
}
fn parse_wall(bytes: &[u8], user: &str, other: bool) -> Result<serde_json::Value> {
    let raw: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| soda_upstream_error("Invalid Soda music wall response"))?;
    let object = raw
        .as_object()
        .ok_or_else(|| soda_upstream_error("Invalid Soda music wall object"))?;
    let mut success = false;
    for value in [
        raw.get("status_code"),
        raw.pointer("/status_info/status_code"),
    ]
    .into_iter()
    .flatten()
    {
        let code = value
            .as_i64()
            .ok_or_else(|| soda_upstream_error("Invalid Soda music wall status"))?;
        if matches!(code, 100_001 | 1_000_016) {
            return Err(wall_auth());
        }
        if code != 0 {
            return Err(soda_upstream_error("Soda music wall request was rejected"));
        }
        success = true;
    }
    if !success {
        return Err(soda_upstream_error(
            "Soda music wall omitted success status",
        ));
    }
    if other
        && raw
            .pointer("/user_brief/id")
            .and_then(serde_json::Value::as_str)
            != Some(user)
    {
        return Err(soda_upstream_error(
            "Soda music wall returned another user's identity",
        ));
    }
    let duration_key = if other {
        "user_listen_duration"
    } else {
        "listen_duration"
    };
    let duration = object
        .get(duration_key)
        .ok_or_else(|| soda_upstream_error("Soda music wall omitted listening metadata"))?;
    if !duration.is_null() {
        let duration = duration
            .as_object()
            .ok_or_else(|| soda_upstream_error("Invalid Soda music wall duration"))?;
        for field in ["total_duration", "week_duration"] {
            if duration
                .get(field)
                .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
            {
                return Err(soda_upstream_error("Invalid Soda music wall duration"));
            }
        }
    }
    let mut content = serde_json::Map::new();
    for field in [
        "tracks",
        "tags",
        "play_button_color",
        "listen_duration",
        "user_listen_duration",
        "reminder",
        "friend_rankings",
        "last_week_friend_rankings",
        "enable_invite",
        "visibility",
        "enable_like",
        "is_visible",
        "my_music_wall_visibility",
    ] {
        if let Some(value) = object.get(field) {
            content.insert(field.into(), value.clone());
        }
    }
    // Preserve native wall content and absent values; the upstream contract does
    // not establish a unit for listening durations. Never label them as seconds.
    let user_ref = ResourceRef::new(Platform::Soda, user)
        .map_err(|_| soda_upstream_error("Invalid Soda music wall user"))?;
    Ok(json!({"user_ref":user_ref,"duration_unit":null,"content":content}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn music_wall_keeps_empty_duration_unknown_and_checks_user_identity() {
        let source =
            json!({"status_code":0,"listen_duration":null,"tracks":null,"unknown_secret":"hidden"});
        let result = parse_wall(&serde_json::to_vec(&source).unwrap(), "1", false).unwrap();
        assert!(result["content"]["listen_duration"].is_null());
        assert!(result["duration_unit"].is_null());
        assert!(!result.to_string().contains("hidden"));
        let user = json!({"status_code":0,"user_listen_duration":{"total_duration":12,"week_duration":2},"user_brief":{"id":"2"}});
        assert!(parse_wall(&serde_json::to_vec(&user).unwrap(), "1", true).is_err());
        assert_eq!(
            parse_wall(&serde_json::to_vec(&user).unwrap(), "2", true).unwrap()["content"]["user_listen_duration"]
                ["total_duration"],
            12
        );
        assert!(parse_wall(br#"{"listen_duration":null}"#, "1", false).is_err());
        assert_eq!(
            parse_wall(br#"{"status_code":1000016}"#, "1", false)
                .err()
                .unwrap()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }
}
