use super::account_read::Read;
use super::*;
use crate::client::user_profile::reject_profile_secrets;
use crate::credential::{error, validate_uid};
use std::time::Duration;
use tuneweave_core::{ErrorCode, UserProfile};

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
            let session = read.native_session().await?;
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

#[cfg(test)]
mod tests;
