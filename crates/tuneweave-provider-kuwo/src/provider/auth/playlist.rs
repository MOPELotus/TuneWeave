use super::*;
use crate::client::native::{
    library::validate_request,
    playlist::{Snapshot, validate_id},
};
#[cfg(test)]
mod collected_tests;
#[cfg(test)]
mod favorite_tests;
#[cfg(test)]
mod tests;

impl KuwoProvider {
    pub(in crate::provider) async fn read_account_playlist(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Snapshot> {
        validate_id(id)?;
        let account = account.unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let result = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, result)?;
        let result = self
            .client
            .fetch_native_account_playlist(&input, Some(id), None, || {
                self.check_selection(account, &selected)
            })
            .await;
        self.finish_selected(account, &selected, result)
    }

    pub(in crate::provider) async fn read_favorite_playlist(
        &self,
        uid: Option<&str>,
        account: Option<&str>,
    ) -> Result<Playlist> {
        Ok(self.read_favorite_snapshot(uid, account).await?.playlist)
    }

    pub(in crate::provider) async fn read_favorite_tracks(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        validate_request(request)?;
        Ok(self
            .read_favorite_snapshot(uid, request.account.as_deref())
            .await?
            .into_page(request))
    }

    async fn read_favorite_snapshot(
        &self,
        uid: Option<&str>,
        account: Option<&str>,
    ) -> Result<Snapshot> {
        let account = account.unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        if uid.is_some_and(|uid| uid != input.user_id()) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Kuwo favorites are available only for the selected account",
            )
            .with_platform(Platform::Kuwo));
        }
        crate::client::native::validate_session_metadata(&input)?;
        let validation = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, validation)?;
        let result = self
            .client
            .fetch_native_account_playlist(
                &input,
                None,
                Some(crate::client::native::library::Section::Favorite),
                || self.check_selection(account, &selected),
            )
            .await;
        self.finish_selected(account, &selected, result)
    }
    pub(in crate::provider) async fn read_account_playlist_tracks(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        validate_request(request)?;
        Ok(self
            .read_account_playlist(id, request.account.as_deref())
            .await?
            .into_page(request))
    }
}
