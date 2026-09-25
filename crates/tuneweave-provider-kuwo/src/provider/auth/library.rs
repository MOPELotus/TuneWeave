use super::*;
use crate::client::native::library::{Section, validate_request};

#[cfg(test)]
mod tests;

impl KuwoProvider {
    pub(in crate::provider) async fn read_account_library(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
        section: Option<Section>,
    ) -> Result<Page<Playlist>> {
        validate_request(request)?;
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        if uid.is_some_and(|uid| uid != input.user_id()) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Kuwo library is available only for the selected account",
            )
            .with_platform(Platform::Kuwo));
        }
        crate::client::native::validate_session_metadata(&input)?;
        let validation = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, validation)?;
        let result = self
            .client
            .fetch_native_library(&input, request, section, || {
                self.check_selection(account, &selected)
            })
            .await;
        self.finish_selected(account, &selected, result)
    }
}
