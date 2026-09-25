use super::*;

impl SodaProvider {
    pub(super) async fn read_account_artist(
        &self,
        id: &str,
        account: Option<&str>,
        deadline: std::time::Duration,
    ) -> Result<Option<ArtistOverview>> {
        if account.is_none() && self.caller_credential.is_none() {
            return Ok(None);
        }
        super::artist_catalog::validate_identity(id)?;
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let initial = source.clone();
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let result = tokio::time::timeout(deadline, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let detail = self.client.pc_artist_detail(id, Some(&source)).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let detail = detail?;
            crate::account::reject_secrets(
                &serde_json::to_value(&detail.overview).map_err(|_| {
                    soda_upstream_error("Soda artist detail metadata could not be encoded")
                })?,
                &[initial],
            )?;
            let updated = detail.credential.ok_or_else(|| {
                soda_upstream_error("Soda account artist detail omitted its credential state")
            })?;
            self.advance_library_credential(&mut source, &mut stored, updated)?;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            Ok(detail.overview)
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda account artist detail exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })??;
        pending.complete();
        Ok(Some(result))
    }
}
