use std::time::Instant;

use reqwest::{
    Method, RequestBuilder,
    header::{ACCEPT, COOKIE, USER_AGENT},
};
use serde::Deserialize;
use tuneweave_core::{
    AccountProfile, ErrorCode, MembershipSummary, Platform, ResourceRef, Result, TuneWeaveError,
    User, UserProfile,
};
use url::Url;

use crate::{
    client::{
        SodaClient, SodaImage, normalize_image, read_bounded_response, soda_http_error,
        soda_upstream_error,
    },
    device::SodaDeviceState,
    login::SodaCredential,
};

const LUNA_PC_ORIGIN: &str = "https://api.qishui.com";
const LUNA_PC_USER_AGENT: &str = "LunaPC/3.7.0(452316191)";
const LUNA_PC_VERSION_NAME: &str = "3.7.0";
const LUNA_PC_VERSION_CODE: &str = "30070000";

pub(crate) fn luna_pc_endpoint(path: &str, device: &SodaDeviceState) -> Result<Url> {
    let mut endpoint = Url::parse(LUNA_PC_ORIGIN)
        .map_err(|_| soda_upstream_error("Soda PC endpoint is invalid"))?;
    endpoint.set_path(path);
    endpoint
        .query_pairs_mut()
        .append_pair("aid", "386088")
        .append_pair("app_name", "luna_pc")
        .append_pair("region", "cn")
        .append_pair("geo_region", "cn")
        .append_pair("os_region", "cn")
        .append_pair("sim_region", "")
        .append_pair("device_id", &device.device_id)
        .append_pair("cdid", "")
        .append_pair("iid", "")
        .append_pair("version_name", LUNA_PC_VERSION_NAME)
        .append_pair("version_code", LUNA_PC_VERSION_CODE)
        .append_pair("channel", "official")
        .append_pair("build_mode", "master")
        .append_pair("network_carrier", "")
        .append_pair("ac", "wifi")
        .append_pair("tz_name", "Asia/Shanghai")
        .append_pair("resolution", "")
        .append_pair("device_platform", "windows")
        .append_pair("device_type", "Windows")
        .append_pair("os_version", "Windows 11 Pro for Workstations")
        .append_pair("fp", &device.device_id);
    Ok(endpoint)
}

pub(crate) fn add_luna_pc_headers(request: RequestBuilder) -> RequestBuilder {
    request
        .header(USER_AGENT, LUNA_PC_USER_AGENT)
        .header(ACCEPT, "application/json")
}

pub(crate) struct SodaAccount {
    pub profile: AccountProfile,
    pub membership: MembershipSummary,
    pub credential: SodaCredential,
}

#[derive(Deserialize)]
struct AccountEnvelope {
    status_code: i64,
    my_info: Option<MyInfo>,
}

#[derive(Deserialize)]
struct MyInfo {
    id: String,
    nickname: Option<String>,
    public_name: Option<String>,
    #[serde(default)]
    larger_avatar_url: SodaImage,
    is_vip: Option<bool>,
    vip_stage: Option<String>,
}

impl SodaClient {
    pub(crate) async fn account(
        &self,
        alias: &str,
        credential: &SodaCredential,
    ) -> Result<SodaAccount> {
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let current_credential = crate::login::token_beat(self, credential).await?;
            let device = self.login_device()?;
            let endpoint = luna_pc_endpoint("/luna/pc/me", &device)?;
            let response = self
                .send_login_request(
                    add_luna_pc_headers(self.login_request(Method::GET, endpoint))
                        .header(COOKIE, current_credential.cookie_header()?),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda account profile").await?;
            let account = parse_account(alias, &current_credential, &body)?;
            Ok(SodaAccount {
                profile: account.profile,
                membership: account.membership,
                credential: account.credential.with_response_cookies(&headers)?,
            })
        }
        .await;
        self.log_upstream_request(
            "account_profile",
            "api.qishui.com",
            "/luna/pc/me",
            http_status,
            started,
            &result,
        );
        result
    }
}

fn parse_account(alias: &str, credential: &SodaCredential, bytes: &[u8]) -> Result<SodaAccount> {
    let envelope: AccountEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| soda_upstream_error("Soda account profile returned invalid data"))?;
    if envelope.status_code == 1_000_016 {
        return Err(authentication_required());
    }
    if envelope.status_code != 0 {
        return Err(soda_upstream_error("Soda account profile was rejected"));
    }
    let info = envelope
        .my_info
        .ok_or_else(|| soda_upstream_error("Soda account profile omitted the account identity"))?;
    let credential = credential.clone().bind_user(&info.id).map_err(|_| {
        soda_upstream_error("Soda account profile returned an invalid or changed identity")
    })?;
    let mut profile = AccountProfile::authenticated(Platform::Soda, alias);
    let mut membership = MembershipSummary {
        user_ref: Some(
            ResourceRef::new(Platform::Soda, &info.id)
                .map_err(|_| soda_upstream_error("Soda membership identity is invalid"))?,
        ),
        active: info.is_vip,
        level: None,
        annual_count: None,
        expires_at: None,
        icon_url: None,
        extensions: Default::default(),
    };
    if let Some(stage) = info.vip_stage {
        if stage.len() > 64 || stage.chars().any(char::is_control) {
            return Err(soda_upstream_error("Soda membership stage is invalid"));
        }
        if !stage.trim().is_empty() {
            membership
                .extensions
                .insert("vip_stage".to_owned(), serde_json::json!(stage));
        }
    }
    profile.user_id = Some(info.id);
    profile.nickname = info
        .nickname
        .filter(|name| !name.trim().is_empty())
        .or_else(|| info.public_name.filter(|name| !name.trim().is_empty()));
    profile.avatar_url = normalize_image(&info.larger_avatar_url);
    Ok(SodaAccount {
        profile,
        membership,
        credential,
    })
}

fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda account session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

pub(crate) fn into_user_profile(profile: AccountProfile) -> Result<UserProfile> {
    let id = profile.user_id.ok_or_else(authentication_required)?;
    Ok(UserProfile {
        user: User {
            resource_ref: ResourceRef::new(Platform::Soda, &id).map_err(|_| {
                soda_upstream_error("Soda account identity cannot form a resource reference")
            })?,
            platform: Platform::Soda,
            id,
            name: profile.nickname.unwrap_or_default(),
            avatar_url: profile.avatar_url,
            signature: None,
            followed: None,
            mutual: None,
            extensions: Default::default(),
        },
        level: None,
        listened_track_count: None,
        playlist_count: None,
        playlist_subscriber_count: None,
        following_count: None,
        follower_count: None,
        event_count: None,
        birthday: None,
        created_at: None,
        background_url: None,
        description: None,
        public_listening_history: None,
        extensions: Default::default(),
    })
}

pub(crate) fn reject_secrets(value: &serde_json::Value, sources: &[SodaCredential]) -> Result<()> {
    let mut secrets = Vec::new();
    for source in sources {
        for pair in source.cookie_header()?.split(';') {
            let (name, value) = pair.trim().split_once('=').ok_or_else(|| {
                soda_upstream_error("Soda account credential could not be inspected")
            })?;
            // Short non-authentication cookies can be ordinary locale/feature flags.
            if matches!(
                name,
                "sessionid"
                    | "sessionid_ss"
                    | "sid_tt"
                    | "sid_guard"
                    | "passport_csrf_token"
                    | "passport_csrf_token_default"
            ) || value.len() >= 16
            {
                secrets.push(value.to_owned());
            }
        }
    }
    let mut values = vec![value];
    while let Some(value) = values.pop() {
        match value {
            serde_json::Value::String(s) => {
                if secrets.iter().any(|secret| {
                    s.contains(secret)
                        || url::form_urlencoded::parse(s.as_bytes())
                            .any(|(k, v)| k.contains(secret) || v.contains(secret))
                }) {
                    return Err(soda_upstream_error(
                        "Soda account metadata exposed session material",
                    ));
                }
            }
            serde_json::Value::Array(v) => values.extend(v),
            serde_json::Value::Object(v) => {
                if v.keys().any(|key| secrets.iter().any(|s| key.contains(s))) {
                    return Err(soda_upstream_error(
                        "Soda account metadata exposed session material",
                    ));
                }
                values.extend(v.values());
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_preserves_unknown_fields_without_inferring_entitlements() {
        for active in [None, Some(false), Some(true)] {
            let mut info = serde_json::json!({"id":"123456"});
            if let Some(active) = active {
                info["is_vip"] = serde_json::json!(active);
            }
            info["vip_stage"] = serde_json::json!("trial");
            let account = parse_account(
                "default",
                &SodaCredential::test_credential("secret"),
                &serde_json::to_vec(&serde_json::json!({"status_code":0,"my_info":info})).unwrap(),
            )
            .unwrap();
            assert_eq!(account.membership.active, active);
            assert_eq!(account.membership.user_ref.unwrap().id(), "123456");
            assert_eq!(account.membership.extensions["vip_stage"], "trial");
            assert!(account.membership.level.is_none());
            assert!(account.membership.expires_at.is_none());
            assert!(account.membership.annual_count.is_none());
        }
        for stage in [
            serde_json::json!("x".repeat(65)),
            serde_json::json!("bad\nstage"),
            serde_json::json!(42),
        ] {
            let bytes = serde_json::to_vec(
                &serde_json::json!({"status_code":0,"my_info":{"id":"123456","vip_stage":stage}}),
            )
            .unwrap();
            assert!(
                parse_account(
                    "default",
                    &SodaCredential::test_credential("secret"),
                    &bytes
                )
                .is_err()
            );
        }
    }

    #[test]
    fn account_identity_is_required_and_bound_to_the_session() {
        let credential = SodaCredential::test_credential("session-secret");
        let account = parse_account(
            "personal",
            &credential,
            br#"{"status_code":0,"my_info":{"id":"123456","nickname":"listener"}}"#,
        )
        .unwrap();
        assert_eq!(account.profile.account, "personal");
        assert_eq!(account.profile.user_id.as_deref(), Some("123456"));
        assert_eq!(account.credential.user_id(), Some("123456"));
        assert!(
            parse_account(
                "personal",
                &account.credential,
                br#"{"status_code":0,"my_info":{"id":"654321"}}"#
            )
            .is_err()
        );
        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_code":0}"#,
            br#"{"status_code":0,"my_info":{"id":""}}"#,
            br#"{"status_code":0,"my_info":{"id":"0123"}}"#,
            br#"{"status_code":9,"my_info":{"id":"123456"}}"#,
        ] {
            assert!(parse_account("personal", &credential, body).is_err());
        }
        let error = parse_account("personal", &credential, br#"{"status_code":1000016}"#)
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn fresh_qr_account_check_beats_token_then_uses_windows_pc_profile_context() {
        let (origin, requests) = crate::test_http::serve(vec![
            crate::test_http::json(
                r#"{"message":"success"}"#,
                Some("sessionid_ss=rotated-session; Path=/; HttpOnly"),
            ),
            crate::test_http::json(
                r#"{"status_code":0,"my_info":{"id":"123456","nickname":"Win account"}}"#,
                None,
            ),
        ])
        .await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let credential = SodaCredential::test_credential("test-session")
            .with_passport_context(
                "7b7e",
                "a1b2c3d4",
                "00000000-0000-4000-8000-000000000000.login",
            )
            .unwrap();
        let account = client.account("default", &credential).await.unwrap();
        assert_eq!(account.profile.user_id.as_deref(), Some("123456"));
        assert!(
            account
                .credential
                .cookie_header()
                .unwrap()
                .contains("sessionid_ss=rotated-session")
        );
        assert!(!account.credential.serialize().unwrap().contains("7b7e"));

        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /passport/token/beat/web/?"));
        assert!(requests[0].contains("scene=boot"));
        assert!(requests[0].contains("version=1.2.14"));
        assert!(requests[0].contains("p_bd=1.0.0.41"));
        assert!(requests[0].to_ascii_lowercase().contains("sodamusic/3.7.0"));
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("cookie: sessionid_ss=test-session")
        );

        assert!(requests[1].starts_with("GET /luna/pc/me?"));
        assert!(requests[1].contains("app_name=luna_pc"));
        assert!(requests[1].contains("version_name=3.7.0"));
        assert!(requests[1].contains("version_code=30070000"));
        assert!(requests[1].contains("device_platform=windows"));
        assert!(requests[1].contains("device_type=Windows"));
        assert!(requests[1].contains("os_version=Windows+11+Pro+for+Workstations"));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("user-agent: lunapc/3.7.0(452316191)")
        );
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("cookie: sessionid_ss=rotated-session")
        );
        let target = requests[1]
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = Url::parse(&format!("http://test.invalid{target}")).unwrap();
        let query = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(query.get("fp"), query.get("device_id"));
    }
}
