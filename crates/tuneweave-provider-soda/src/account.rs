use std::time::Instant;

use reqwest::{
    Method,
    header::{ACCEPT, COOKIE},
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
        soda_network_error, soda_upstream_error,
    },
    login::SodaCredential,
};

const ACCOUNT_ENDPOINT: &str = "https://api.qishui.com/luna/pc/me?aid=386088";

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
            let endpoint = Url::parse(ACCOUNT_ENDPOINT)
                .map_err(|_| soda_upstream_error("Soda account endpoint is invalid"))?;
            let response = self
                .login_request(Method::GET, endpoint)
                .header(ACCEPT, "application/json")
                .header(COOKIE, credential.cookie_header()?)
                .send()
                .await
                .map_err(soda_network_error)?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda account profile").await?;
            let account = parse_account(alias, credential, &body)?;
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
}
