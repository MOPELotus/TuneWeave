use super::account_read::Read;
use super::*;
use crate::client::user_profile::reject_profile_secrets;
use crate::credential::{error, validate_uid};
use std::time::Duration;
use tuneweave_core::{AccountProfile, ErrorCode, Extensions, ResourceRef, User, UserProfile};

const DEADLINE: Duration = Duration::from_secs(45);

impl MiguProvider {
    pub(super) async fn read_user_profile(
        &self,
        id: &str,
        backend: tuneweave_core::UserProfileBackend,
        account: Option<&str>,
    ) -> Result<UserProfile> {
        if backend != tuneweave_core::UserProfileBackend::Modern {
            return Err(TuneWeaveError::unsupported(
                Platform::Migu,
                Capability::UserProfileLegacy,
            ));
        }
        validate_uid(id)?;
        let mut read = Read::new(self, account)?;
        if read.current.user_id() != id {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu user profiles are available only for the selected account",
            ));
        }
        let result = tokio::time::timeout(DEADLINE, async {
            let (account_profile, native_session) = read.profile_and_native_session().await?;
            let Some(session) = native_session else {
                return h5_user_profile(account_profile);
            };
            let reply = self
                .client
                .account_h5_token(read.current.token(), &session)
                .await;
            let candidate = read.accept(reply).await??;
            read.secrets.push(candidate.clone());
            let auth = self
                .client
                .validate_native_token(candidate, read.current.user_id())
                .await;
            read.check()?;
            let auth = auth?;
            let profile = self.client.native_user_profile(&auth).await;
            read.check()?;
            let profile = profile?;
            reject_profile_secrets(&profile, &read.secrets)?;
            // Native display fields cannot substitute for an independently
            // checked PACM identity and the original selected login generation.
            read.start().await?;
            reject_profile_secrets(&profile, &read.secrets)?;
            Ok(profile)
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu user profile exceeded its total deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result)
    }
}

fn h5_user_profile(profile: AccountProfile) -> Result<UserProfile> {
    if !profile.authenticated || profile.platform != Platform::Migu {
        return Err(migu_upstream_error(
            "Migu H5 account profile is not authenticated",
        ));
    }
    let id = profile
        .user_id
        .ok_or_else(|| migu_upstream_error("Migu H5 profile omitted its user identity"))?;
    validate_uid(&id)
        .map_err(|_| migu_upstream_error("Migu H5 profile omitted its user identity"))?;
    let resource_ref = ResourceRef::new(Platform::Migu, &id)
        .map_err(|_| migu_upstream_error("Migu profile identity is invalid"))?;
    Ok(UserProfile {
        user: User {
            resource_ref,
            platform: Platform::Migu,
            id,
            name: profile.nickname.unwrap_or_default(),
            avatar_url: profile.avatar_url,
            signature: None,
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
        birthday: None,
        created_at: None,
        background_url: None,
        description: None,
        public_listening_history: None,
        extensions: Extensions::from([("backend".into(), json!("official_h5_user_info"))]),
    })
}

#[cfg(test)]
mod tests;
