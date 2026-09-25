use super::*;
use tuneweave_core::{PodcastEpisode, PodcastEpisodeStream};

#[cfg(test)]
mod tests;

impl KuwoClient {
    /// Resolves a native anchor programme to its independently identified music resource.
    pub async fn podcast_episode(&self, id: &str, account: Option<&str>) -> Result<PodcastEpisode> {
        let music = request_episode(id, account)?;
        tokio::time::timeout(Duration::from_secs(45), async {
            let track = self.track_detail(music).await?;
            let album = episode_album(&track)?;
            let show = self.podcast(&format!("anchor:{album}"), None).await?;
            map_episode(track, show)
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo podcast episode detail exceeded the total time budget",
            )
            .with_platform(Platform::Kuwo))
        })
    }

    /// Resolves a fresh anonymous playback authorization; no audio bytes are downloaded.
    pub async fn podcast_episode_stream(
        &self,
        id: &str,
        request: &StreamRequest,
    ) -> Result<PodcastEpisodeStream> {
        request_episode(id, request.account.as_deref())?;
        validate_media_request(request)?;
        tokio::time::timeout(Duration::from_secs(60), async {
            let episode = self.podcast_episode(id, None).await?;
            let audio = episode.audio.as_ref().ok_or_else(invalid)?;
            let stream = self.stream(audio, request).await?;
            Ok(PodcastEpisodeStream {
                episode_ref: episode.resource_ref.clone(),
                audio_ref: audio.resource_ref.clone(),
                stream,
                extensions: Extensions::from([("episode".into(), json!(episode))]),
            })
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo podcast episode playback exceeded the total time budget",
            )
            .with_platform(Platform::Kuwo))
        })
    }
}

fn request_episode<'a>(id: &'a str, account: Option<&str>) -> Result<&'a str> {
    id.strip_prefix("episode:")
        .and_then(canonical_positive_decimal)
        .filter(|_| account.is_none())
        .ok_or_else(|| {
            kuwo_invalid_request("Kuwo episode requires episode:<positive music ID> and no account")
        })
}

fn episode_album(track: &Track) -> Result<String> {
    if track
        .extensions
        .get("is_starred")
        .and_then(serde_json::Value::as_str)
        != Some("1")
        || track
            .extensions
            .get("content_type")
            .and_then(serde_json::Value::as_str)
            != Some("0")
    {
        return Err(invalid());
    }
    let album = track
        .album
        .as_ref()
        .and_then(|a| a.resource_ref.as_ref())
        .ok_or_else(invalid)?;
    if album.platform() != Platform::Kuwo || canonical_positive_decimal(album.id()).is_none() {
        return Err(invalid());
    }
    Ok(album.id().to_owned())
}

fn map_episode(track: Track, show: Podcast) -> Result<PodcastEpisode> {
    let music = canonical_media_track_id(&track)?;
    let album = episode_album(&track)?;
    if show.resource_ref.to_string() != format!("kuwo:anchor:{album}") {
        return Err(invalid());
    }
    let mut episode = PodcastEpisode::new(
        ResourceRef::new(Platform::Kuwo, format!("episode:{music}")).map_err(|_| invalid())?,
        track.name.clone(),
    );
    episode.podcast_ref = Some(show.resource_ref);
    episode.cover_url = track.album.as_ref().and_then(|a| a.cover_url.clone());
    episode.creator = track.artists.first().map(|a| CreatorSummary {
        resource_ref: a.resource_ref.clone(),
        name: a.name.clone(),
        avatar_url: None,
    });
    episode.duration_ms = track.duration_ms;
    episode.published_at = track
        .extensions
        .get("release_date")
        .and_then(serde_json::Value::as_str)
        .map(catalog::date)
        .transpose()?
        .flatten();
    episode.serial_number = track
        .extensions
        .get("track_number")
        .and_then(serde_json::Value::as_str)
        .map(|value| {
            canonical_positive_decimal(value)
                .and_then(|v| v.parse().ok())
                .ok_or_else(invalid)
        })
        .transpose()?;
    episode
        .extensions
        .insert("source_music_id".into(), json!(music));
    episode
        .extensions
        .insert("backend".into(), json!("current_web_anchor_episode"));
    // The album description is not an episode description, and catalogue flags
    // are not evidence of this caller's purchased/paid/subscribed state.
    episode.audio = Some(track);
    Ok(episode)
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo podcast episode did not identify an ordinary native anchor programme")
}
