use super::*;
use std::collections::BTreeMap;
use tuneweave_core::{VideoDetailRequest, VideoStream, VideoStreamRequest};

impl KugouProvider {
    pub(super) async fn read_video_streams(
        &self,
        ids: &[String],
        request: &VideoStreamRequest,
    ) -> Result<Vec<VideoStream>> {
        if request.resolution == 0 {
            return Err(kugou_invalid_request(
                "KuGou video resolution must be positive",
            ));
        }
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.read_native_video_streams(ids, request).await;
        }
        // Validate public catalogue identities and batch sizes before any request,
        // and restore the exact original references.
        let details = self
            .read_videos(
                ids,
                &VideoDetailRequest {
                    kind: request.kind,
                    account: request.account.clone(),
                },
            )
            .await?;
        let mut found = BTreeMap::new();
        let mut streams = Vec::with_capacity(details.len());
        for detail in details {
            if !found.contains_key(&detail.video.id) {
                let stream = self
                    .client
                    .public_video_stream(&detail, request.resolution)
                    .await?;
                found.insert(detail.video.id.clone(), stream);
            }
            let mut stream = found.get(&detail.video.id).cloned().ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "Missing resolved KuGou video stream",
                )
                .with_platform(Platform::Kugou)
            })?;
            stream.video_ref = detail.video.resource_ref;
            streams.push(stream);
        }
        Ok(streams)
    }
}

impl KugouProvider {
    async fn read_native_video_streams(
        &self,
        ids: &[String],
        request: &VideoStreamRequest,
    ) -> Result<Vec<VideoStream>> {
        super::videos::validate_video_ids(ids, request.kind)?;
        let mut read = self
            .begin_native_read(request.account.as_deref().unwrap_or("default"), None)
            .await?;
        let result = async {
            let vip_type = read.vip_type.ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "KuGou account profile omitted the video privilege membership field",
                )
                .with_platform(Platform::Kugou)
            })?;
            let session = read.session()?.clone();
            let details = self
                .read_video_catalogue(ids, request.kind, || self.check_account_read(&mut read))
                .await?;
            // Deduplication is local to this selected login and this one metadata snapshot.
            let mut found = BTreeMap::new();
            let mut streams = Vec::with_capacity(details.len());
            for detail in details {
                if !found.contains_key(&detail.video.id) {
                    let stream = self
                        .client
                        .native_video_stream(
                            &detail,
                            request.resolution,
                            &session,
                            vip_type,
                            || self.check_account_read(&mut read),
                        )
                        .await?;
                    found.insert(detail.video.id.clone(), stream);
                }
                let mut stream = found.get(&detail.video.id).cloned().ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::InternalError,
                        "Missing resolved KuGou account video stream",
                    )
                    .with_platform(Platform::Kugou)
                })?;
                stream.video_ref = detail.video.resource_ref;
                streams.push(stream);
            }
            Ok(streams)
        }
        .await;
        self.finish_account_read(read, result)
    }
}

#[cfg(test)]
mod tests;
