use super::*;
use std::collections::BTreeMap;
use tuneweave_core::{
    VideoDetail, VideoDetailRequest, VideoResourceKind, VideoStats, VideoStream, VideoStreamRequest,
};

fn validate(id: &str, request: &VideoDetailRequest) -> Result<()> {
    if request.kind != VideoResourceKind::Mv
        || request.account.is_some()
        || id
            .parse::<u64>()
            .ok()
            .is_none_or(|n| n == 0 || n.to_string() != id)
    {
        return Err(kuwo_invalid_request(
            "Kuwo public MV metadata requires kind=mv, a canonical positive music ID and no account",
        ));
    }
    Ok(())
}
impl KuwoProvider {
    pub(super) async fn read_video_streams(
        &self,
        ids: &[String],
        request: &VideoStreamRequest,
    ) -> Result<Vec<VideoStream>> {
        if ids.is_empty() || ids.len() > 100 || request.resolution == 0 {
            return Err(kuwo_invalid_request(
                "Kuwo MV playback requires 1–100 IDs and a positive resolution preference",
            ));
        }
        let metadata_request = VideoDetailRequest {
            kind: request.kind,
            account: request.account.clone(),
        };
        for id in ids {
            validate(id, &metadata_request)?;
        }
        // This cache is local to one batch; a later request reads current permissions.
        let mut resolved = BTreeMap::new();
        let mut output = Vec::with_capacity(ids.len());
        for id in ids {
            if !resolved.contains_key(id) {
                resolved.insert(
                    id.clone(),
                    self.client.mv_stream(id, request.resolution).await?,
                );
            }
            output.push(
                resolved
                    .get(id)
                    .cloned()
                    .ok_or_else(|| kuwo_upstream_error("Kuwo MV batch lost a resolved entry"))?,
            );
        }
        Ok(output)
    }
    pub(super) async fn read_video(
        &self,
        id: &str,
        request: &VideoDetailRequest,
    ) -> Result<VideoDetail> {
        validate(id, request)?;
        self.client.mv_detail(id).await
    }
    pub(super) async fn read_video_stats(
        &self,
        id: &str,
        request: &VideoDetailRequest,
    ) -> Result<VideoStats> {
        let detail = self.read_video(id, request).await?;
        Ok(VideoStats {
            video_ref: detail.video.resource_ref,
            kind: VideoResourceKind::Mv,
            view_count: detail.video.play_count,
            liked: None,
            favorited: None,
            coins_contributed: None,
            danmaku_count: None,
            like_count: None,
            coin_count: None,
            favorite_count: None,
            comment_count: None,
            share_count: None,
            extensions: Extensions::from([("backend".into(), json!("current_web_mv_music_info"))]),
        })
    }
}
