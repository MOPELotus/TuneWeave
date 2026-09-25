use super::*;
use crate::credential::{authentication_required, error, validate_uid};
use tuneweave_core::{ErrorCode, MembershipSummary};

impl MiguProvider {
    pub(super) async fn read_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
        detailed: bool,
    ) -> Result<MembershipSummary> {
        if let Some(id) = id {
            validate_uid(id)?;
        }
        let account = account.unwrap_or("default");
        let (mut current, mut stored) = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        if id.is_some_and(|id| id != current.user_id()) {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu membership is available only for the selected account",
            ));
        }
        let original = current.clone();
        let result: Result<MembershipSummary> = async {
            // The member center also serves anonymous product cards. Verify identity
            // before reading it and verify every returned token before accepting data.
            self.verify_account_step(account, &mut current, &mut stored, original.token())
                .await?;
            let center = self
                .client
                .account_membership(current.token(), current.user_id())
                .await?;
            self.accept_read(&current, stored.as_ref(), &current)?;
            self.verify_account_step(account, &mut current, &mut stored, &center.token)
                .await?;
            let mut summary = center.data?;
            let icons = self.client.account_member_icons(current.token()).await?;
            self.accept_read(&current, stored.as_ref(), &current)?;
            self.verify_account_step(account, &mut current, &mut stored, &icons.token)
                .await?;
            let icons = icons.data?;
            summary.icon_url = icons.iter().find_map(|icon| icon.icon_url.clone());
            summary.extensions.insert(
                "member_icons".into(),
                serde_json::to_value(icons).map_err(|_| {
                    error(ErrorCode::UpstreamError, "Migu member icons are invalid")
                })?,
            );
            if detailed {
                let identities = self
                    .client
                    .account_media_identities(current.token(), current.user_id())
                    .await?;
                self.accept_read(&current, stored.as_ref(), &current)?;
                self.verify_account_step(account, &mut current, &mut stored, &identities.token)
                    .await?;
                summary.extensions.insert(
                    "media_member_identities".into(),
                    serde_json::to_value(identities.data?).map_err(|_| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu media identities are invalid",
                        )
                    })?,
                );
            }
            Ok(summary)
        }
        .await;
        self.finish_account_read(&original, &current, stored.as_ref(), result)
    }
}

#[cfg(test)]
mod tests;
