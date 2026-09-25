use super::account::profile_image;
use super::account_download::NativeAuthorization;
use super::account_media::reject_url_secret;
use super::*;
use tuneweave_core::{User, UserProfile};

// The official UserInfoItem DTO. Deliberately omit account names, bound phone
// numbers, pass IDs, session IDs and untyped provider fields.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    user_id: String,
    nick_name: Option<String>,
    signature: Option<String>,
    birthday: Option<String>,
    middle_icon: Option<String>,
    small_icon: Option<String>,
    big_icon: Option<String>,
    icon: Option<String>,
    bgpic: Option<String>,
}

fn text(value: Option<String>, limit: usize, multiline: bool) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(value) if value.is_empty() => Ok(None),
        Some(value)
            if value.len() <= limit
                && !value.chars().any(|ch| {
                    ch.is_control() && !(multiline && matches!(ch, '\n' | '\r' | '\t'))
                }) =>
        {
            Ok(Some(value))
        }
        Some(_) => Err(migu_upstream_error("Migu profile display field is invalid")),
    }
}

impl MiguClient {
    pub(crate) async fn native_user_profile(
        &self,
        auth: &NativeAuthorization,
    ) -> Result<UserProfile> {
        // The shared reader requires the official ACK and same-UID root
        // userInfoItem before any display data can leave the client.
        let mut value = self.native_profile_response(auth).await?;
        let profile: Profile = serde_json::from_value(value["userInfoItem"].take())
            .map_err(|_| migu_upstream_error("Migu native user profile is invalid"))?;
        let avatar = [
            profile.middle_icon,
            profile.small_icon,
            profile.big_icon,
            profile.icon,
        ]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty());
        Ok(UserProfile {
            user: User {
                resource_ref: ResourceRef::new(Platform::Migu, &profile.user_id)
                    .map_err(|_| migu_upstream_error("Migu profile identity is invalid"))?,
                platform: Platform::Migu,
                id: profile.user_id,
                name: text(profile.nick_name, 256, false)?.unwrap_or_default(),
                avatar_url: avatar.as_deref().and_then(profile_image),
                signature: text(profile.signature, 4096, true)?,
                followed: None,
                mutual: None,
                extensions: Extensions::default(),
            },
            level: None,
            listened_track_count: None,
            playlist_count: None,
            playlist_subscriber_count: None,
            following_count: None,
            follower_count: None,
            event_count: None,
            // The official UI displays this string verbatim (its editor uses
            // yyyy/MM/dd). Do not invent a timestamp, timezone or missing date.
            birthday: text(profile.birthday, 32, false)?,
            created_at: None,
            background_url: profile.bgpic.as_deref().and_then(profile_image),
            description: None,
            public_listening_history: None,
            extensions: Extensions::from([("backend".into(), json!("official_native_user_info"))]),
        })
    }
}

pub(crate) fn reject_profile_secrets(profile: &UserProfile, secrets: &[String]) -> Result<()> {
    for value in [
        Some(profile.user.id.as_str()),
        Some(profile.user.name.as_str()),
        profile.user.signature.as_deref(),
        profile.birthday.as_deref(),
        profile.user.avatar_url.as_deref(),
        profile.background_url.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if secrets
            .iter()
            .any(|secret| !secret.is_empty() && value.contains(secret))
        {
            return Err(migu_upstream_error(
                "Migu profile display data contains authorization material",
            ));
        }
    }
    for url in [
        profile.user.avatar_url.as_deref(),
        profile.background_url.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        for secret in secrets {
            reject_url_secret(url, secret)?;
        }
    }
    Ok(())
}
