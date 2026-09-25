use super::*;
use crate::client::native::{library, submissions};
use tuneweave_core::PlaylistSubmission;

#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod recommendation_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod write_tests;

impl KuwoProvider {
    pub(in crate::provider) async fn delete_native_submission_records(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistSubmissionRecordDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistSubmissionRecordDeleteResult> {
        use crate::client::native::{management::submission_delete::Progress, playlist};
        playlist::validate_id(id)?;
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let mut progress = Progress::default();
        let result = tokio::time::timeout(self.client.submission_limits().budget, async {
            let identity = self.client.validate_native_session(&input).await;
            self.check_selection(account, &selected)?;
            identity?;
            self.client
                .perform_native_submission_record_delete(&input, id, &mut progress, || {
                    self.check_selection(account, &selected)
                })
                .await
        })
        .await
        .map_err(|_| submissions::timeout())
        .and_then(|v| v);
        self.finish_selected(account, &selected, result)
            .map_err(|e| progress.failure(e))
    }

    pub(in crate::provider) async fn submit_native_playlist(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistSubmissionRequest,
    ) -> Result<tuneweave_core::PlaylistSubmissionResult> {
        use crate::client::native::management::contribution::{self, Progress};
        contribution::validate(id, request)?;
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        contribution::validate_secrets(request, &input)?;
        let mut progress = Progress::default();
        let result = tokio::time::timeout(self.client.submission_limits().budget, async {
            let identity = self.client.validate_native_session(&input).await;
            self.check_selection(account, &selected)?;
            identity?;
            self.client
                .perform_native_submission(&input, id, request, &mut progress, || {
                    self.check_selection(account, &selected)
                })
                .await
        })
        .await
        .map_err(|_| submissions::timeout())
        .and_then(|v| v);
        self.finish_selected(account, &selected, result)
            .map_err(|e| progress.failure(e))
    }

    pub(in crate::provider) async fn read_playlist_submissions(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PlaylistSubmission>> {
        library::validate_request(request)?;
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let result = tokio::time::timeout(self.client.submission_limits().budget, async {
            let validation = self.client.validate_native_session(&input).await;
            self.check_selection(account, &selected)?;
            validation?;
            self.client
                .fetch_native_submissions(&input, request, || {
                    self.check_selection(account, &selected)
                })
                .await
        })
        .await
        .map_err(|_| submissions::timeout())
        .and_then(|v| v);
        self.finish_selected(account, &selected, result)
    }
}
