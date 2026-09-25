use super::*;
use crate::client::videos::{DETAIL_PAGE_SIZE, canonical_id};
use std::collections::BTreeMap;
use tuneweave_core::{VideoDetail, VideoDetailRequest, VideoResourceKind};

impl KugouProvider {
    pub(super) async fn read_videos(
        &self,
        ids: &[String],
        request: &VideoDetailRequest,
    ) -> Result<Vec<VideoDetail>> {
        validate_video_ids(ids, request.kind)?;
        if request.account.is_some() || self.caller_credential.is_some() {
            let mut read = self
                .begin_native_read(request.account.as_deref().unwrap_or("default"), None)
                .await?;
            let result = self
                .read_video_catalogue(ids, request.kind, || self.check_account_read(&mut read))
                .await;
            return self.finish_account_read(read, result);
        }
        self.require_public_source()?;
        self.read_video_catalogue(ids, request.kind, || Ok(()))
            .await
    }

    pub(super) async fn read_video_catalogue(
        &self,
        ids: &[String],
        kind: VideoResourceKind,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Vec<VideoDetail>> {
        let requested_ids = ids;
        let ids = validate_video_ids(ids, kind)?;
        // Preserve requested ordering and duplicates, while fetching each identity once.
        let mut seen = BTreeSet::new();
        let unique = ids
            .iter()
            .filter(|id| seen.insert((*id).clone()))
            .cloned()
            .collect::<Vec<_>>();
        let mut found = BTreeMap::new();
        for batch in unique.chunks(DETAIL_PAGE_SIZE) {
            let details = self.client.public_video_details(batch, kind).await;
            check()?;
            for detail in details? {
                found.insert(detail.video.id.clone(), detail);
            }
        }
        ids.iter()
            .zip(requested_ids)
            .map(|(id, requested_id)| {
                let mut detail = found.get(id).cloned().ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "KuGou video batch omitted a requested identity",
                    )
                    .with_platform(Platform::Kugou)
                })?;
                // Preserve the exact reference required by HTTP batch validation.
                detail.video.resource_ref =
                    tuneweave_core::ResourceRef::new(Platform::Kugou, requested_id.clone())
                        .map_err(|_| kugou_invalid_request("Invalid KuGou video reference"))?;
                Ok(detail)
            })
            .collect()
    }
}

pub(super) fn validate_video_ids(ids: &[String], kind: VideoResourceKind) -> Result<Vec<String>> {
    if ids.is_empty() || ids.len() > 100 {
        return Err(kugou_invalid_request(
            "KuGou video detail batches require 1 to 100 IDs",
        ));
    }
    let prefix = match kind {
        VideoResourceKind::Mv => "mv:",
        VideoResourceKind::Video => "video:",
    };
    ids.iter().map(|id| {
        let id = id.strip_prefix(prefix).unwrap_or(id);
        if !canonical_id(id) {
            return Err(kugou_invalid_request("KuGou video IDs must be canonical positive decimals with an optional matching kind prefix"));
        }
        Ok(id.to_owned())
    }).collect()
}

#[cfg(test)]
mod tests;
