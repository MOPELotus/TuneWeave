use super::account_read::Read;
use super::*;
use crate::client::account_media::{reject_url_secret, selected_tones, validate_request};
use crate::credential::error;
use std::time::Duration;
use tuneweave_core::{ErrorCode, Quality};

impl MiguProvider {
    pub(super) async fn download_account_track(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaDownload> {
        validate_request(track, request)?;
        let mut read = Read::new(self, request.account.as_deref())?;
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            let session = read.native_session().await?;
            let reply = self
                .client
                .account_h5_token(read.current.token(), &session)
                .await;
            let candidate = read.accept(reply).await??;
            read.secrets.push(candidate.clone());
            let auth = self
                .client
                .validate_native_token(candidate, read.current.user_id())
                .await;
            read.check()?;
            let auth = auth?;
            let fresh = read.track(&track.id).await?;
            let tones = selected_tones(&fresh, request)?;
            let mut denied = None;
            for tone in tones {
                read.check()?;
                let reply = self
                    .client
                    .account_download(&auth, &fresh, request, tone)
                    .await;
                read.check()?;
                match reply {
                    Ok(download) => {
                        // Verify the selected PACM account again after upstream authorization.
                        read.start().await?;
                        if let Some(url) = download.url.as_deref() {
                            for secret in &read.secrets {
                                reject_url_secret(url, secret)?;
                            }
                        }
                        return Ok(download);
                    }
                    Err(failure)
                        if failure.code == ErrorCode::PermissionDenied
                            && request.quality == Quality::Auto
                            && request.bitrate.is_none() =>
                    {
                        denied = Some(failure)
                    }
                    Err(failure) => return Err(failure),
                }
            }
            Err(denied.unwrap_or_else(|| {
                error(
                    ErrorCode::PermissionDenied,
                    "Migu did not authorize any requested download format",
                )
            }))
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu account download exceeded its total deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result)
    }

    pub(super) async fn download_account_content(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<tuneweave_core::AudioContent> {
        validate_request(track, request)?;
        if request.account.is_none() && self.caller_credential.is_none() {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "Migu download content requires an explicitly selected account",
            ));
        }
        if !matches!(
            request.quality,
            Quality::Auto
                | Quality::Low
                | Quality::Standard
                | Quality::Higher
                | Quality::High
                | Quality::Lossless
                | Quality::Hires
        ) {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "Migu download content supports PQ/HQ MP3 and SQ/ZQ24 FLAC renditions only",
            ));
        }
        let mut read = Read::new(self, request.account.as_deref())?;
        let result = tokio::time::timeout(Duration::from_secs(120), async {
            let session = read.native_session().await?;
            let reply = self
                .client
                .account_h5_token(read.current.token(), &session)
                .await;
            let candidate = read.accept(reply).await??;
            read.secrets.push(candidate.clone());
            let auth = self
                .client
                .validate_native_token(candidate, read.current.user_id())
                .await;
            read.check()?;
            let auth = auth?;
            let fresh = read.track(&track.id).await?;
            let duration = fresh
                .duration_ms
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    error(
                        ErrorCode::UpstreamError,
                        "Migu download metadata omitted the complete track duration",
                    )
                })?;
            let tones = selected_tones(&fresh, request)?;
            let mut denied = None;
            for tone in tones
                .into_iter()
                .filter(|tone| matches!(*tone, "PQ" | "HQ" | "SQ" | "ZQ24"))
            {
                read.check()?;
                let reply = self
                    .client
                    .account_download_content(&auth, &fresh, request, tone)
                    .await;
                read.check()?;
                let grant = match reply {
                    Ok(grant) => grant,
                    Err(failure)
                        if failure.code == ErrorCode::PermissionDenied
                            && request.quality == Quality::Auto
                            && request.bitrate.is_none() =>
                    {
                        denied = Some(failure);
                        continue;
                    }
                    Err(failure) => return Err(failure),
                };
                let url = grant.media.url.as_deref().ok_or_else(|| {
                    error(
                        ErrorCode::UpstreamError,
                        "Migu download grant omitted its URL",
                    )
                })?;
                for secret in &read.secrets {
                    reject_url_secret(url, secret)?;
                }
                let bytes = self.client.fetch_download_content(&grant).await;
                read.check()?;
                let mut bytes = bytes?;
                if let Some(key) = &grant.key {
                    let key = crate::client::mg3d::derive(key);
                    for (index, chunk) in bytes.chunks_mut(64 * 1024).enumerate() {
                        crate::client::mg3d::decode(chunk, &key, index * 64 * 1024);
                        tokio::task::yield_now().await;
                        read.check()?;
                    }
                }
                let (content_type, suffix) = if matches!(tone, "SQ" | "ZQ24") {
                    let expected_bits = if tone == "ZQ24" { 24 } else { 16 };
                    crate::client::mg3d::flac::inspect(&bytes, duration, expected_bits, || {
                        read.check()
                    })
                    .await?;
                    ("audio/flac", "flac")
                } else {
                    crate::client::mg3d::inspect_mp3(&bytes, duration, || read.check())?;
                    ("audio/mpeg", "mp3")
                };
                read.start().await?;
                // Recheck every rotating secret after the last profile request.
                for secret in &read.secrets {
                    reject_url_secret(url, secret)?;
                }
                return Ok(tuneweave_core::AudioContent {
                    track_ref: fresh.resource_ref,
                    bytes,
                    content_type: content_type.into(),
                    filename: format!("{}.{suffix}", fresh.id),
                    trial: None,
                });
            }
            Err(denied.unwrap_or_else(|| {
                error(
                    ErrorCode::PermissionDenied,
                    "Migu did not authorize a supported content download",
                )
            }))
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu content download exceeded its total deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result)
    }
}

#[cfg(test)]
mod tests;
