use super::*;

impl SodaProvider {
    pub(super) async fn read_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<MembershipSummary> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        if id.is_some_and(|id| {
            id.is_empty()
                || id.len() > 32
                || id.starts_with('0')
                || !id.bytes().all(|b| b.is_ascii_digit())
        }) {
            return Err(soda_invalid_request(
                "Soda membership user ID must be a canonical positive decimal",
            ));
        }
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let denied = || {
            TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Soda membership is available only for the selected account",
            )
            .with_platform(Platform::Soda)
        };
        if id.is_some_and(|id| source.user_id().is_some_and(|uid| uid != id)) {
            return Err(denied());
        }
        let mut seen = vec![source.clone()];
        let result = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let verified = verified?;
            if id.is_some_and(|id| verified.credential.user_id() != Some(id)) {
                return Err(denied());
            }
            seen.push(verified.credential.clone());
            self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
            let membership = self.client.commerce_membership(&source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let membership = membership?;
            seen.push(membership.credential.clone());
            // Commerce has no verified UID field. Even an unchanged Cookie is
            // independently rechecked before the account-specific result is released.
            let verified = self.client.account(alias, &membership.credential).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let verified = verified?;
            seen.push(verified.credential.clone());
            crate::account::reject_secrets(
                &serde_json::to_value(&membership.summary)
                    .map_err(|_| soda_upstream_error("Soda membership could not be encoded"))?,
                &seen,
            )?;
            self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            Ok(membership.summary)
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda membership exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })?;
        let summary = result?;
        pending.complete();
        Ok(summary)
    }
}
