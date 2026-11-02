use super::*;
impl SodaProvider {
    pub(super) async fn read_music_wall(
        &self,
        user: Option<&str>,
        account: Option<&str>,
    ) -> Result<serde_json::Value> {
        if user.is_some_and(|id| {
            id.len() > 64 || crate::client::canonical_positive_decimal(id) != Some(id)
        }) {
            return Err(soda_invalid_request(
                "Soda music wall requires a canonical user ID",
            ));
        }
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let result = tokio::time::timeout(std::time::Duration::from_secs(45), async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let wall = self.client.account_music_wall(user, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let (wall, updated) = wall?;
            self.advance_library_credential(&mut source, &mut stored, updated)?;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            Ok(wall)
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda music wall exceeded its time budget",
            )
            .with_platform(Platform::Soda)
        })?;
        let wall = result?;
        pending.complete();
        Ok(wall)
    }
}
