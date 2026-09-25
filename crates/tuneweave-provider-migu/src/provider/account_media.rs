use super::account_read::Read;
use super::*;
use crate::client::account_media::{reject_url_secret, selected_tones, validate_request};
use crate::credential::error;
use std::time::Duration;
use tuneweave_core::{ErrorCode, Quality, ResourceRef};

const DEADLINE: Duration = Duration::from_secs(45);

fn timeout_error(_: tokio::time::error::Elapsed) -> TuneWeaveError {
    error(
        ErrorCode::UpstreamTimeout,
        "Migu account media exceeded its total deadline",
    )
}

impl MiguProvider {
    pub(super) async fn read_account_track(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Track> {
        parse_content_id(id)?;
        let mut read = Read::new(self, account)?;
        let result = tokio::time::timeout(DEADLINE, async {
            read.start().await?;
            read.track(id).await
        })
        .await
        .map_err(timeout_error)
        .and_then(|result| result);
        read.finish(result)
    }

    pub(super) async fn search_account_tracks(&self, query: &SearchQuery) -> Result<Page<Track>> {
        let mut public = query.clone();
        public.account = None;
        validate_search_query(&public)?;
        let mut read = Read::new(self, query.account.as_deref())?;
        let result = tokio::time::timeout(DEADLINE, async {
            read.start().await?;
            let mut page = self.search_tracks_public(&public, || read.check()).await?;
            read.check()?;
            read.mark(&mut page.pagination.extensions);
            for track in &mut page.items {
                read.mark(&mut track.extensions);
            }
            Ok(page)
        })
        .await
        .map_err(timeout_error)
        .and_then(|result| result);
        read.finish(result)
    }

    pub(super) async fn stream_account_track(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaStream> {
        validate_request(track, request)?;
        let mut read = Read::new(self, request.account.as_deref())?;
        let result = tokio::time::timeout(DEADLINE, async {
            read.start().await?;
            let fresh = read.track(&track.id).await?;
            let response = self
                .client
                .account_media_rights(read.current.token(), &fresh.id)
                .await;
            let rights = read.accept(response).await??;
            if !rights.can_listen && !rights.limit_length {
                return Err(error(
                    ErrorCode::PermissionDenied,
                    "Migu did not authorize account playback",
                ));
            }
            let tones = selected_tones(&fresh, request)?;
            let mut denied = None;
            for tone in tones {
                read.check()?;
                let response = self
                    .client
                    .account_media_play(read.current.token(), &fresh, &rights, request, tone)
                    .await;
                match read.accept(response).await? {
                    Ok(stream) => {
                        for token in &read.secrets {
                            reject_url_secret(&stream.url, token)?;
                        }
                        return Ok(stream);
                    }
                    Err(failure)
                        if failure.code == ErrorCode::PermissionDenied
                            && request.quality == Quality::Auto
                            && request.bitrate.is_none() =>
                    {
                        denied = Some(failure);
                    }
                    Err(failure) => return Err(failure),
                }
            }
            Err(denied.unwrap_or_else(|| {
                error(
                    ErrorCode::PermissionDenied,
                    "Migu did not authorize any requested audio format",
                )
            }))
        })
        .await
        .map_err(timeout_error)
        .and_then(|result| result);
        read.finish(result)
    }

    pub(super) async fn account_track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        parse_content_id(id)?;
        let bitrate = match request.bitrate {
            999_000 => None,
            1..=320_000 => Some(request.bitrate),
            _ => {
                return Err(migu_invalid_request(
                    "Migu account availability supports bitrates 1 through 320000 or 999000 for automatic quality",
                ));
            }
        };
        let track_ref = ResourceRef::new(Platform::Migu, id)
            .map_err(|_| migu_invalid_request("Invalid Migu track identity"))?;
        let track = Track::new(track_ref.clone(), "Migu account track");
        let stream_request = StreamRequest {
            quality: Quality::Auto,
            bitrate,
            account: request.account.clone(),
            ..StreamRequest::default()
        };
        let result = self.stream_account_track(&track, &stream_request).await;
        let mut extensions = Extensions::new();
        extensions.insert("backend".into(), json!("pc_account_listen_v2"));
        match result {
            Ok(stream) => {
                let playable = stream.trial.is_none();
                extensions.insert("actual_quality".into(), json!(stream.actual_quality));
                extensions.insert("trial".into(), json!(stream.trial));
                Ok(TrackAvailability {
                    track_ref,
                    playable,
                    requested_bitrate: request.bitrate,
                    actual_bitrate: stream.bitrate,
                    platform_code: Some(0),
                    message: if playable {
                        "ok".into()
                    } else {
                        "Migu only permits an account preview".into()
                    },
                    extensions,
                })
            }
            // Keep credential rotation on the scoped provider; HTTP can return it
            // even when the supported availability answer is false.
            Err(failure) if failure.code == ErrorCode::PermissionDenied => Ok(TrackAvailability {
                track_ref,
                playable: false,
                requested_bitrate: request.bitrate,
                actual_bitrate: None,
                platform_code: None,
                message: "Migu did not authorize the requested account playback".into(),
                extensions,
            }),
            Err(failure) => Err(failure),
        }
    }
}

#[cfg(test)]
pub(super) mod tests;
