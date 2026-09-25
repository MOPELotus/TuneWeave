use super::*;
use tuneweave_core::MembershipSummary;

#[cfg(test)]
mod tests;

impl KuwoProvider {
    pub(in crate::provider) async fn read_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<MembershipSummary> {
        let account = account.unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        if id.is_some_and(|id| id != input.user_id()) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Kuwo membership is available only for the selected account",
            )
            .with_platform(Platform::Kuwo));
        }
        crate::client::native::validate_session_metadata(&input)?;
        let validation = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, validation)?;
        let result = self.client.fetch_native_membership(&input).await;
        self.finish_selected(account, &selected, result)
    }
}
