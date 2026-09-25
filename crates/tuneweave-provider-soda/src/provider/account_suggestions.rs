use super::*;

impl SodaProvider {
    pub(super) async fn read_account_suggestions(
        &self,
        request: &SearchSuggestionRequest,
        budget: std::time::Duration,
    ) -> Result<SearchSuggestionList> {
        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let result = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let verified = verified?;
            self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
            let result = self
                .client
                .account_search_suggestions(&request.query, &source, request.client)
                .await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let (list, updated) = result?;
            self.advance_library_credential(&mut source, &mut stored, updated)?;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            Ok(list)
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda account suggestions exceeded their total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })?;
        let list = result?;
        pending.complete();
        Ok(list)
    }
}
