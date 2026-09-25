use super::*;
use crate::{
    account::media::Behavior,
    client::account_media::{self, Selection},
};
use tuneweave_core::{Quality, ResourceRef, TrackAvailability, TrackAvailabilityRequest};

// Only explicit upstream media denials permit another quality. Parser, transport,
// authentication and local format errors must not be hidden by another request.
fn media_denial(error: &TuneWeaveError) -> bool {
    error.code == ErrorCode::PermissionDenied
        && error
            .details
            .get("additional_verification_required")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        && (matches!(
            error
                .details
                .get("platform_status")
                .and_then(serde_json::Value::as_i64),
            Some(2 | 3)
        ) || error
            .details
            .get("platform_code")
            .and_then(serde_json::Value::as_i64)
            == Some(35002)
            || error
                .details
                .get("web_media_denied")
                .and_then(serde_json::Value::as_bool)
                == Some(true))
}

impl KugouProvider {
    pub(super) async fn account_media_candidates(
        &self,
        query: &SearchQuery,
    ) -> Result<Page<Track>> {
        let mut catalogue_query = query.clone();
        catalogue_query.account = None;
        validate_search_query(&catalogue_query)?;
        let read = self
            .begin_media_read(query.account.as_deref().unwrap_or("default"))
            .await?;
        // Matching uses the public catalogue. No account tokens go to song_search_v2;
        // candidates acquire account media rights only through account_media_stream.
        let public = Self::from_client(self.client.clone());
        let result = public.search(&catalogue_query).await.map(|mut page| {
            page.pagination
                .extensions
                .insert("catalogue_scope".into(), json!("public"));
            page
        });
        self.finish_account_read(read, result)
    }

    pub(super) async fn account_media_track(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Track> {
        let id = parse_album_audio_id(id)?;
        let read = self.begin_media_read(account.unwrap_or("default")).await?;
        // This is catalogue metadata, not an account permission/quality assertion.
        let result = self.client.track_detail(id).await;
        self.finish_account_read(read, result)
    }

    pub(super) async fn account_media_stream(
        &self,
        track: &Track,
        request: &StreamRequest,
        behavior: Behavior,
    ) -> Result<MediaStream> {
        let id = account_media::validate_track(track)?;
        account_media::validate_request(request)?;
        let mut read = self
            .begin_media_read(request.account.as_deref().unwrap_or("default"))
            .await?;
        let result = async {
            if read.web_session().is_some() && behavior == Behavior::Download {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "KuGou Web playback does not provide an independent download authorization",
                )
                .with_platform(Platform::Kugou));
            }
            // Never authorize arbitrary hashes supplied in a caller's Track extensions.
            let catalogue = self.client.track_detail(id).await?;
            self.check_account_read(&mut read)?;
            if let Some(session) = read.web_session().cloned() {
                return self
                    .client
                    .web_media_stream(&session, catalogue, request, || {
                        self.check_account_read(&mut read)
                    })
                    .await;
            }
            let mut selection = Selection::new(catalogue, request)?;
            let mut preferred_trial = None;
            loop {
                // Fresh consumed grants for every asset, within the same AccountRead.
                // A user-level denial is terminal; it is not specific to a quality.
                let user = self
                    .client
                    .native_user_authorization(read.session()?)
                    .await?;
                self.check_account_read(&mut read)?;
                let response = async {
                    let grant = self
                        .client
                        .native_song_authorization(read.session()?, user, id, selection.hash())
                        .await?;
                    self.check_account_read(&mut read)?;
                    self.client
                        .native_audio_tracker(
                            read.session()?,
                            grant,
                            selection.album_id,
                            selection.quality(),
                            behavior,
                        )
                        .await
                }
                .await;
                // Also check failed attempts before issuing a lower-quality request.
                self.check_account_read(&mut read)?;
                let response = match response {
                    Ok(response) => response,
                    Err(error) if media_denial(&error) => {
                        if let Some(lower) = selection.lower()? {
                            selection = lower;
                            continue;
                        }
                        return preferred_trial.ok_or(error);
                    }
                    Err(error) => return Err(error),
                };
                // Parse the explicit trial before deciding whether to try a lower
                // asset. The wire request still uses its original play/download behavior.
                let lower = selection.lower();
                let stream = selection.map(response, Behavior::Play)?;
                if stream.trial.is_none() {
                    return Ok(stream);
                }
                if behavior == Behavior::Play && preferred_trial.is_none() {
                    preferred_trial = Some(stream);
                }
                match lower? {
                    Some(value) => selection = value,
                    None => {
                        return preferred_trial.ok_or_else(|| {
                            TuneWeaveError::new(
                                ErrorCode::PermissionDenied,
                                "KuGou only authorized a trial; full download is unavailable",
                            )
                            .with_platform(Platform::Kugou)
                        });
                    }
                }
            }
        }
        .await;
        self.finish_account_read(read, result)
    }

    pub(super) async fn account_media_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        let id = parse_album_audio_id(id)?;
        let mut selection = StreamRequest {
            account: request.account.clone(),
            ..Default::default()
        };
        match request.bitrate {
            TrackAvailabilityRequest::DEFAULT_BITRATE => selection.quality = Quality::Master,
            1..=320_000 => selection.bitrate = Some(request.bitrate),
            _ => {
                return Err(kugou_invalid_request(
                    "KuGou availability bitrate must be 1–320000, or 999000 for the highest supported quality",
                ));
            }
        }
        if request.account.is_none() && self.caller_credential.is_none() {
            return Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                "KuGou availability requires a selected account",
            )
            .with_platform(Platform::Kugou));
        }
        let reference = ResourceRef::new(Platform::Kugou, id.to_string())
            .map_err(|_| kugou_invalid_request("KuGou media track identity is invalid"))?;
        let track = Track::new(reference.clone(), "");
        let result = self
            .account_media_stream(&track, &selection, Behavior::Play)
            .await;
        let mut availability = TrackAvailability {
            track_ref: reference,
            playable: false,
            requested_bitrate: request.bitrate,
            actual_bitrate: None,
            platform_code: None,
            message: "KuGou did not authorize full playback".into(),
            extensions: Extensions::from([("authorization_behavior".into(), json!("play"))]),
        };
        match result {
            Ok(stream) => {
                availability.playable = stream.trial.is_none();
                availability.actual_bitrate = stream.bitrate;
                availability.platform_code = Some(0);
                availability.message = if availability.playable {
                    "Full playback authorized"
                } else {
                    "Only trial playback authorized"
                }
                .into();
                availability
                    .extensions
                    .insert("actual_quality".into(), json!(stream.actual_quality));
                if let Some(trial) = stream.trial {
                    availability.extensions.insert("trial".into(), json!(trial));
                }
            }
            Err(error) if media_denial(&error) => {
                availability.platform_code = error
                    .details
                    .get("platform_code")
                    .and_then(serde_json::Value::as_i64);
                availability.extensions.insert(
                    "platform_status".into(),
                    error.details["platform_status"].clone(),
                );
            }
            Err(error) => return Err(error),
        }
        Ok(availability)
    }

    pub(super) async fn account_media_download(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaDownload> {
        let stream = self
            .account_media_stream(track, request, Behavior::Download)
            .await?;
        Ok(MediaDownload {
            track_ref: stream.resolved_track,
            platform: Platform::Kugou,
            available: true,
            url: Some(stream.url),
            headers: stream.headers,
            expires_at: stream.expires_at,
            format: stream.format,
            codec: stream.codec,
            bitrate: stream.bitrate,
            size: stream.size,
            duration_ms: stream.duration_ms,
            requested_quality: stream.requested_quality,
            actual_quality: stream.actual_quality,
            platform_code: Some(1),
            fee: None,
            message: None,
            extensions: Extensions::from([("authorization_behavior".into(), json!("download"))]),
        })
    }
}

#[cfg(test)]
mod tests;
