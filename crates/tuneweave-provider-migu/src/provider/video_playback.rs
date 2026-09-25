use super::account_read::Read;
use super::*;
use crate::client::account_media::reject_url_secret;
use std::collections::BTreeMap;
use tuneweave_core::{ErrorCode, MiguNativeMvStreamRequest, VideoStream, VideoStreamRequest};

const DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);
impl MiguProvider {
    pub(super) async fn read_native_mv_stream(
        &self,
        id: &str,
        request: &MiguNativeMvStreamRequest,
    ) -> Result<VideoStream> {
        self.read_native_mv_stream_bounded(id, request, DEADLINE)
            .await
    }

    async fn read_native_mv_stream_bounded(
        &self,
        id: &str,
        request: &MiguNativeMvStreamRequest,
        deadline: std::time::Duration,
    ) -> Result<VideoStream> {
        if !crate::client::videos::valid_id(id) {
            return Err(migu_invalid_request(
                "Migu MV IDs must be canonical positive integers",
            ));
        }
        if request.account.is_some() || self.caller_credential.is_some() {
            let mut read = Read::new(self, request.account.as_deref())?;
            let result = tokio::time::timeout(deadline, async {
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
                let duration = self.client.native_mv_duration(id).await;
                read.check()?;
                let duration = duration?;
                let grant = self
                    .client
                    .native_mv_authorization(id, duration, request.format, Some(&auth))
                    .await;
                read.check()?;
                let grant = grant?;
                // The native CDN userId is opaque, not the music UID. Bind the
                // request via the validated token/CE, never by decoding this URL.
                for secret in &read.secrets {
                    reject_url_secret(&grant.url, secret)?;
                }
                let stream = self
                    .client
                    .native_mv_manifest(grant, id, duration, request.format)
                    .await;
                read.check()?;
                let mut stream = stream?;
                // Keep the same selected music identity through the last network
                // boundary, including any PACM rotation learned after the grant.
                read.start().await?;
                for secret in &read.secrets {
                    reject_url_secret(stream.url.as_deref().unwrap_or_default(), secret)?;
                }
                read.mark(&mut stream.extensions);
                stream
                    .extensions
                    .insert("authorization_scope".into(), json!("selected_account"));
                Ok(stream)
            })
            .await
            .unwrap_or_else(|_| {
                Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "Migu native MV playback exceeded its total deadline",
                )
                .with_platform(Platform::Migu))
            });
            return read.finish(result);
        }
        let operation = self.client.native_mv_stream(id, request.format);
        let stream = tokio::time::timeout(deadline, operation)
            .await
            .unwrap_or_else(|_| {
                Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "Migu native MV playback exceeded its total deadline",
                )
                .with_platform(Platform::Migu))
            })?;
        if stream.video_ref.id() != id || stream.platform != Platform::Migu {
            return Err(migu_upstream_error(
                "Migu native MV returned a stream for another resource",
            ));
        }
        Ok(stream)
    }

    pub(super) async fn read_mv_stream(
        &self,
        id: &str,
        request: &VideoStreamRequest,
    ) -> Result<VideoStream> {
        self.read_mv_stream_bounded(id, request, DEADLINE).await
    }
    async fn read_mv_stream_bounded(
        &self,
        id: &str,
        request: &VideoStreamRequest,
        deadline: std::time::Duration,
    ) -> Result<VideoStream> {
        Ok(self
            .read_mv_streams_bounded(&[id.to_owned()], request, deadline)
            .await?
            .remove(0))
    }
    pub(super) async fn read_mv_streams(
        &self,
        ids: &[String],
        request: &VideoStreamRequest,
    ) -> Result<Vec<VideoStream>> {
        self.read_mv_streams_bounded(ids, request, DEADLINE).await
    }
    async fn read_mv_streams_bounded(
        &self,
        ids: &[String],
        request: &VideoStreamRequest,
        deadline: std::time::Duration,
    ) -> Result<Vec<VideoStream>> {
        if ids.is_empty() || ids.len() > 100 {
            return Err(migu_invalid_request(
                "Migu MV stream batch requires 1 to 100 IDs",
            ));
        }
        for id in ids {
            crate::client::video_playback::validate(id, request)?;
        }
        let mut selected = if request.account.is_some() || self.caller_credential.is_some() {
            Some(Read::new(self, request.account.as_deref())?)
        } else {
            None
        };
        let result = tokio::time::timeout(deadline, async {
            if let Some(read) = &mut selected {
                read.start().await?;
                read.check()?;
            }
            let mut completed = BTreeMap::<String, VideoStream>::new();
            let mut streams = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(stream) = completed.get(id) {
                    streams.push(stream.clone());
                    continue;
                }
                let source = self.client.mv_play_source(id, request).await;
                if let Some(read) = &selected {
                    read.check()?;
                }
                let source = source?;
                let reply = self
                    .client
                    .mv_play_grant(selected.as_ref().map(|r| r.current.token()), &source)
                    .await;
                let url = if let Some(read) = &mut selected {
                    read.accept(reply).await??
                } else {
                    reply?.data?
                };
                if let Some(read) = &selected {
                    for secret in &read.secrets {
                        reject_url_secret(&url, secret)?;
                    }
                    let parsed = url::Url::parse(&url)
                        .map_err(|_| migu_upstream_error("Invalid Migu MV URL"))?;
                    if parsed
                        .query_pairs()
                        .any(|(k, v)| k == "userId" && !v.is_empty() && v != read.current.user_id())
                    {
                        return Err(migu_upstream_error(
                            "Migu MV URL belongs to another account",
                        ));
                    }
                    read.check()?;
                }
                let result = self.client.mv_manifest(&url, &source, request).await;
                if let Some(read) = &selected {
                    read.check()?;
                }
                let mut stream = result?;
                if let Some(read) = &selected {
                    for secret in &read.secrets {
                        reject_url_secret(stream.url.as_deref().unwrap_or_default(), secret)?;
                    }
                    read.mark(&mut stream.extensions);
                    stream
                        .extensions
                        .insert("authorization_scope".into(), json!("selected_account"));
                } else {
                    stream
                        .extensions
                        .insert("authorization_scope".into(), json!("anonymous"));
                }
                completed.insert(id.clone(), stream.clone());
                streams.push(stream);
            }
            // A later grant may rotate the session again. Recheck every earlier
            // URL against all credentials learned before the batch is delivered.
            if let Some(read) = &selected {
                for stream in &streams {
                    for secret in &read.secrets {
                        reject_url_secret(stream.url.as_deref().unwrap_or_default(), secret)?;
                    }
                }
            }
            Ok(streams)
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Migu MV playback exceeded its total deadline",
            )
            .with_platform(Platform::Migu))
        });
        if let Some(read) = &mut selected {
            read.finish(result)
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests;
