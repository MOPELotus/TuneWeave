use super::*;
use crate::client::native::media::{self, Action, Outcome};
use tuneweave_core::ResourceRef;

#[cfg(test)]
mod content_tests;
#[cfg(test)]
mod dtsx_tests;
#[cfg(test)]
mod sing_along_tests;
#[cfg(test)]
mod spatial_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vinyl_tests;

impl KuwoProvider {
    pub(in crate::provider) async fn read_native_content(
        &self,
        track: &Track,
        request: &StreamRequest,
        action: Action,
        budget: Duration,
    ) -> Result<tuneweave_core::AudioContent> {
        media::validate_request(request)?;
        crate::client::canonical_media_track_id(track)?;
        if request.account.is_none() && self.caller_credential.is_none() {
            return Err(authentication_required());
        }
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        let result = self
            .client
            .fetch_native_content(&input, track, request, action, budget, || {
                self.check_selection(account, &selected)
            })
            .await;
        self.finish_selected(account, &selected, result)
    }

    pub(in crate::provider) async fn read_native_media_track(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Track> {
        let id = parse_music_id(id)?;
        let account = account.unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let operation = async {
            self.check_selection(account, &selected)?;
            let validation = self.client.validate_native_session(&input).await;
            self.check_selection(account, &selected)?;
            validation?;
            // Only catalogue metadata uses the anonymous Web session. It receives
            // no native credentials and cannot authorize playback or downloads.
            let result = self
                .client
                .track_detail_guarded(id, || self.check_selection(account, &selected))
                .await;
            self.check_selection(account, &selected)?;
            result.map(|mut track| {
                track.playable = None;
                track.available_qualities.clear();
                track
                    .extensions
                    .insert("catalogue_scope".into(), json!("public"));
                track
                    .extensions
                    .insert("account_media_rights_separate".into(), json!(true));
                track
            })
        };
        let result = tokio::time::timeout(media::BUDGET, operation)
            .await
            .unwrap_or_else(|_| {
                Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "Kuwo account catalogue read exceeded its time budget",
                )
                .with_platform(Platform::Kuwo)
                .retryable(false))
            });
        self.finish_selected(account, &selected, result)
    }

    async fn read_native_media(
        &self,
        id: &str,
        request: &StreamRequest,
        action: Action,
        budget: Duration,
    ) -> Result<Outcome> {
        media::validate_request(request)?;
        let id = parse_music_id(id)?;
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        let result = self
            .client
            .fetch_native_media(&input, id, request, action, budget, || {
                self.check_selection(account, &selected)
            })
            .await;
        self.finish_selected(account, &selected, result)
    }
    pub(in crate::provider) async fn read_native_stream(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaStream> {
        let id = crate::client::canonical_media_track_id(track)?;
        self.read_native_media(id, request, Action::Play, media::BUDGET)
            .await?
            .stream(track, request)
    }
    pub(in crate::provider) async fn read_native_download(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaDownload> {
        let id = crate::client::canonical_media_track_id(track)?;
        Ok(self
            .read_native_media(id, request, Action::Download, media::BUDGET)
            .await?
            .download(track, request))
    }
    pub(in crate::provider) async fn read_native_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        let id = parse_music_id(id)?;
        let stream = media::availability_request(request)?;
        let outcome = self
            .read_native_media(id, &stream, Action::Play, media::BUDGET)
            .await?;
        Ok(outcome.availability(
            ResourceRef::new(Platform::Kuwo, id)
                .map_err(|_| kuwo_invalid_request("Kuwo native media ID is invalid"))?,
            request,
        ))
    }
}
